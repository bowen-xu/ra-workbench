//! Shared submission admission for paper workflows.
use super::*;

/// Admit a persisted paper-analysis submission and project the shared runner's
/// normalized events into its application-owned conversation. The legacy
/// structured candidate adapter still owns output parsing, but no longer owns
/// turn admission, event sequencing, cancellation, or terminal history.
pub(in crate::ai) async fn call_prepared_vision_submission(
    store: AppStore,
    sessions: Arc<AgentSessionService>,
    prepared: PreparedAgentModel,
    input: ProviderInput,
    conversation_id: Uuid,
    request_id: Uuid,
    cancellation: CancellationSignal,
) -> Result<VisionGenerationResponse, AiError> {
    let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
    let frozen = match submissions.get(conversation_id, request_id).await {
        Ok(record) => Some(record),
        Err(research_agent_core::submission::SubmissionError::NotFound) => None,
        Err(_) => return Err(AiError::TemporaryUnavailable),
    };
    let workflow = super::workflow::prepare(
        &store,
        conversation_id,
        request_id,
        frozen.as_ref().map(|record| &record.input),
    )
    .await?;
    call_with_workflow(
        store,
        sessions,
        prepared,
        input,
        conversation_id,
        request_id,
        cancellation,
        submissions,
        workflow,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn call_with_workflow(
    store: AppStore,
    sessions: Arc<AgentSessionService>,
    prepared: PreparedAgentModel,
    input: ProviderInput,
    conversation_id: Uuid,
    request_id: Uuid,
    cancellation: CancellationSignal,
    submissions: SqliteSubmissionStore,
    workflow: Option<Box<dyn crate::ai::generation_hooks::PreparedPaperWorkflow>>,
) -> Result<VisionGenerationResponse, AiError> {
    ensure_submission(
        &store,
        &submissions,
        &prepared,
        &input,
        conversation_id,
        request_id,
        workflow.as_deref(),
    )
    .await?;
    let has_images = workflow.is_none()
        && input
            .messages
            .iter()
            .any(|message| !message.images.is_empty());
    let (send, receive) = tokio::sync::oneshot::channel();
    let worker = sessions.clone();
    let skill_store = store.clone();
    let admission = sessions
        .start_submission(
            &submissions,
            ConversationId::from_uuid(conversation_id),
            request_id,
            move |record, _, task_cancellation| async move {
                {
                    let db = skill_store.connection().map_err(|_| RunError::EventSink)?;
                    crate::agent::skills::injection::validate_mentions(
                        &skill_store,
                        &db,
                        &record.input.user_input,
                        &record.input.context,
                    )
                    .map_err(|_| RunError::ToolRejected)?;
                    crate::agent::skills::steering::freeze_grants(&db, &skill_store, &record)
                        .map_err(|_| RunError::ToolRejected)?;
                }
                if crate::agent::skills::application::validate(&record.input).is_err() {
                    let _ = send.send(Err(AiError::InvalidInput(
                        "论文分析 shared submission 的 skill 身份无效".to_owned(),
                    )));
                    return Err(RunError::ToolRejected);
                }
                if record.input.model != *prepared.identity() {
                    let error = AiError::InvalidInput(
                        "论文分析模型身份与冻结 submission 不一致".to_owned(),
                    );
                    let _ = send.send(Err(error));
                    return Err(RunError::Model(ModelError::InvalidRequest));
                }
                let combined = crate::agent::cancellation::AgentCancellation::default();
                let combined_signal = combined.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = cancellation.cancelled() => combined_signal.cancel(),
                        _ = task_cancellation.cancelled() => combined_signal.cancel(),
                    }
                });
                let skill_tools = match crate::agent::skills::application::tools(
                    skill_store.clone(),
                    record.clone(),
                )
                .await
                {
                    Ok(tools) => tools,
                    Err(_) => {
                        let _ = send.send(Err(AiError::TemporaryUnavailable));
                        return Err(RunError::ToolUnavailable);
                    }
                };
                let skill_catalog=skill_tools.catalog(65536).map_err(|_|RunError::ToolUnavailable)?.model_text;
                let skill_definitions = skill_tools.definitions();
                let source_tools = Arc::new(crate::agent::source_tools::OwnedSubmissionTools::new(
                    record.clone(),
                ));
                let source_definitions = source_tools.definitions();
                let mut host_definitions = source_definitions.clone();
                host_definitions.extend(skill_definitions.clone());
                let host_tools = Arc::new(crate::agent::composed_tools::ComposedToolExecutor::new(
                    source_tools,
                    source_definitions,
                    Some(Arc::new(skill_tools)),
                    skill_definitions,
                )) as Arc<dyn ToolExecutor>;
                let mcp_runtime = worker.mcp_runtime().await;
                let (tool_executor, tool_definitions) = match mcp_runtime {
                    Some(runtime) => (
                        Some(
                            Arc::new(crate::agent::composed_tools::ComposedToolExecutor::new(
                                host_tools,
                                host_definitions.clone(),
                                Some(runtime.clone()),
                                runtime.definitions(),
                            )) as Arc<dyn ToolExecutor>,
                        ),
                        {
                            let mut definitions = host_definitions.clone();
                            definitions.extend(runtime.definitions());
                            definitions
                        },
                    ),
                    None => (Some(host_tools), host_definitions),
                };
                let registration = research_agent_core::tool_registry::OwnedToolRegistration::new(
                    tool_definitions,
                    tool_executor.ok_or(RunError::ToolUnavailable)?,
                );
                let registration = crate::agent::context_session::resource_tools::registration(
                    skill_store.clone(),
                    record.clone(),
                    registration,
                );
                let registration = crate::agent::host_execution::registration(
                    skill_store.clone(),
                    record.clone(),
                    worker.clone(),
                    registration,
                )
                .await?;
                let (tool_definitions, executor) = registration.into_parts();
                let tool_executor = Some(executor);
                let mut input = input;
                if let Some(workflow) = &workflow
                    && let Err(error) = workflow.decorate(&mut input, &record.input) {
                        let _ = send.send(Err(error));
                        return Err(RunError::Model(ModelError::InvalidRequest));
                }
                if skill_catalog!="[]" {
                    let system=input.system_text.get_or_insert_with(String::new);
                    system.push_str("\nAvailable skill catalog (metadata only; load exact identity on demand):\n");
                    system.push_str(&skill_catalog);
                }
                let prompts = record
                    .input
                    .skill_instructions
                    .iter()
                    .map(|prompt| prompt.render())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if !prompts.is_empty() {
                    let system = input.system_text.get_or_insert_with(String::new);
                    system.push_str("\n\n");
                    system.push_str(&prompts);
                }
                let result = call_prepared_model(
                    prepared,
                    input,
                    workflow.is_none(),
                    8_000,
                    has_images,
                    VisionRunOptions {
                        stream_sink: None,
                        cancellation: None,
                        durable: Some(DurableProjection {
                            service: worker,
                            conversation_id: ConversationId::from_uuid(record.conversation_id),
                            request_id: record.request_id,
                        }),
                        turn_id: Some(record.turn_id),
                        agent_cancellation: Some(combined),
                        tool_executor,
                        tool_definitions,
                    },
                )
                .await;
                let result = match result {
                    Ok(response) => match &workflow {
                        Some(workflow) => workflow.complete(
                            &skill_store,
                            record.conversation_id,
                            record.request_id,
                            response,
                        ).await,
                        None => Ok(response),
                    },
                    Err(error) => Err(error),
                };
                let run_result = result.as_ref().map(|_| ()).map_err(map_ai_error);
                let _ = send.send(result);
                run_result
            },
        )
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?;
    let owned = match admission {
        SubmissionAdmission::Started(owned) => owned,
        SubmissionAdmission::AlreadyStarted { .. } => {
            return Err(AiError::InvalidInput(
                "论文分析 shared turn 已经启动".to_owned(),
            ));
        }
    };
    resolve_submission_completion(owned.task.wait().await, receive.await.ok())
}

/// Compatibility bridge for legacy structured text runs. Selected domain policy
/// validates candidates; the Core repository owns durable candidate persistence.
/// Admission, event sequencing, cancellation and terminal history remain shared.
pub(in crate::ai) async fn call_prepared_text_submission(
    store: AppStore,
    sessions: Arc<AgentSessionService>,
    prepared: PreparedAgentModel,
    input: ProviderInput,
    conversation_id: Uuid,
    request_id: Uuid,
    cancellation: CancellationSignal,
) -> Result<VisionGenerationResponse, AiError> {
    call_prepared_vision_submission(
        store,
        sessions,
        prepared,
        input,
        conversation_id,
        request_id,
        cancellation,
    )
    .await
}

/// Both image and text compatibility entrances must admit the same immutable
/// input. A previously admitted production submission is never reconstructed.
async fn ensure_submission(
    store: &AppStore,
    submissions: &SqliteSubmissionStore,
    prepared: &PreparedAgentModel,
    input: &ProviderInput,
    conversation_id: Uuid,
    request_id: Uuid,
    workflow: Option<&dyn crate::ai::generation_hooks::PreparedPaperWorkflow>,
) -> Result<(), AiError> {
    match submissions.get(conversation_id, request_id).await {
        Ok(_) => return Ok(()),
        Err(research_agent_core::submission::SubmissionError::NotFound) => {}
        Err(_) => return Err(AiError::TemporaryUnavailable),
    }
    if workflow.is_none() && input.messages.is_empty() {
        return Err(AiError::InvalidInput(
            "画像生成缺少受管理的文件任务；不能退回旧摘录请求".into(),
        ));
    }
    let text = workflow
        .map(|workflow| workflow.prompt_text().to_owned())
        .unwrap_or_else(|| legacy_submission_text(input));
    let (skills, skill_instructions) = crate::agent::skills::injection::freeze_application(
        store.agent_configuration_directory(),
        &text,
        &[],
    )
    .map_err(|_| AiError::TemporaryUnavailable)?;
    let mut submission = SubmissionInput {
        remote_compaction_v2: None,
        skill_instructions,
        user_input: workflow
            .map(|workflow| workflow.prompt_user_input().to_vec())
            .unwrap_or_default(),
        text,
        model: prepared.identity().clone(),
        reasoning_effort: prepared.reasoning_effort().map(str::to_owned),
        service_tier: None,
        context: workflow
            .map(|workflow| workflow.grants().to_vec())
            .unwrap_or_default(),
        instructions: Vec::new(),
        instruction_configs: Vec::new(),
        skills,
    };
    if let Some(workflow) = workflow {
        crate::ai::structured_generation::bind_task_skill(
            store.agent_configuration_directory(),
            workflow.kind(),
            &mut submission,
        )?;
    }
    submissions
        .put(conversation_id, request_id, submission)
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?;
    Ok(())
}

#[cfg(all(test, debug_assertions, feature = "development-data"))]
#[path = "submissions/tests.rs"]
mod tests;
