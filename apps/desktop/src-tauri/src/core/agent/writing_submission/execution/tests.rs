use super::super::tests::{fixture, request};
use super::*;
use crate::agent::{
    model::OutputTokenParameter, repository::SqliteConversationStore, tasks::TaskCompletion,
};
use research_agent_core::runner::{RunLimits, ToolExecutor};
use research_agent_core::{model::*, protocol::TurnOutcome};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};
#[path = "tests/child_cleanup.rs"]
mod child_cleanup;
#[path = "tests/child_submission_write.rs"]
mod child_submission_write;
#[cfg(target_os = "macos")]
#[path = "tests/compaction_hooks.rs"]
mod compaction_hooks;
#[cfg(target_os = "macos")]
#[path = "tests/native_acceptance.rs"]
mod native_acceptance;
#[path = "tests/profile.rs"]
mod profile;
#[path = "tests/queued.rs"]
mod queued;
#[path = "tests/queued_paused.rs"]
mod queued_paused;
#[cfg(target_os = "macos")]
#[path = "tests/shell.rs"]
mod shell;
#[cfg(unix)]
#[path = "tests/skills.rs"]
mod skills;
struct Transport(AtomicUsize);
struct Stream(VecDeque<ModelStreamEvent>);
async fn persisted_failure(
    sessions: &AgentSessionService,
    record: &SubmissionRecord,
    store: &AppStore,
) -> String {
    let projection = sessions
        .open(ConversationId::from_uuid(record.conversation_id))
        .await
        .unwrap();
    let db = store.connection().unwrap();
    let mut statement=db.prepare("select json_extract(payload_json,'$.payload.error') from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 and json_extract(payload_json,'$.payload.type')='modelRequestFailed' order by ordinal").unwrap();
    let errors = statement
        .query_map(
            rusqlite::params![
                record.conversation_id.to_string(),
                record.turn_id.to_string()
            ],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    format!(
        "projection={:?}, request_failures={errors:?}",
        projection
            .turn(record.turn_id)
            .map(|turn| (&turn.failure, &turn.attempts))
    )
}

struct SteeringTransport {
    inner: Transport,
    queue: crate::agent::repository::steering::SqliteSteeringStore,
    input: research_agent_core::steering::SteeringInput,
}
impl ModelTransport for SteeringTransport {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        use research_agent_core::steering::SteeringStore;
        Box::pin(async move {
            if self.inner.0.load(Ordering::SeqCst) == 0 {
                self.queue
                    .enqueue(self.input.clone())
                    .await
                    .map_err(|_| ModelError::Unavailable)?;
            } else {
                assert!(
                    matches!(request.messages.last(), Some(ModelMessage::User { content: parts, source_item_id: Some(source) }) if *source==self.input.input_id && matches!(parts.as_slice(), [ContentPart::Text(text)] if text == &self.input.text))
                );
            }
            self.inner.start(request).await
        })
    }
}

#[tokio::test]
async fn input_accepted_during_model_request_is_consumed_in_the_same_turn() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let prepared = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let record = prepared.record;
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let transport = Arc::new(SteeringTransport {
        inner: Transport(AtomicUsize::new(0)),
        queue: crate::agent::repository::steering::SqliteSteeringStore::new(
            f.store.shared_agent_connection(),
        ),
        input: research_agent_core::steering::SteeringInput {
            skill_instructions: vec![],
            user_input: vec![],
            scope: research_agent_core::steering::SteeringScope {
                conversation_id: record.conversation_id,
                turn_id: record.turn_id,
            },
            input_id: Uuid::new_v4(),
            text: "Add an evidence caveat".into(),
        },
    });
    let admission = service
        .start_transport(
            sessions.clone(),
            record.clone(),
            profile(record.input.model.clone()),
            transport.clone(),
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(
        owned.task.wait().await,
        TaskCompletion::Completed,
        "persisted failure: {}",
        persisted_failure(&sessions, &record, &f.store).await
    );
    assert_eq!(transport.inner.0.load(Ordering::SeqCst), 2);
    let projection = sessions
        .open(ConversationId::from_uuid(record.conversation_id))
        .await
        .unwrap();
    assert_eq!(projection.turn_order().len(), 1);
    let turn = projection.turn(record.turn_id).unwrap();
    assert_eq!(
        turn.items[&transport.input.input_id].text,
        transport.input.text
    );
    assert!(turn.items[&transport.input.input_id].completed);
}
impl ModelStream for Stream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}
impl ModelTransport for Transport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(Box::new(Stream(
                vec![
                    ModelStreamEvent::Started {
                        response_id: "fixture".into(),
                        actual_model: request.model,
                    },
                    ModelStreamEvent::TextDelta("Answer".into()),
                    ModelStreamEvent::Completed {
                        reason: FinishReason::Stop,
                        end_turn: None,
                    },
                ]
                .into(),
            )) as Box<dyn ModelStream>)
        })
    }
}
fn profile(model: ModelIdentity) -> WritingExecutionProfile {
    WritingExecutionProfile {
        hook_context_bytes: 512,
        reasoning_summary: None,
        chat_developer_model: None,
        compact_prompt: None,
        token_budget_settings: None,
        comp_hash: None,
        context_budget_policy:
            crate::agent::execution_profile::NativeContextBudgetPolicy::REVIEWED_DEFAULT,
        compaction_policy:
            crate::agent::execution_profile::compaction::NativeCompactionPolicy::Text,
        request_policy: Default::default(),
        model,
        capabilities: Transport(AtomicUsize::new(0)).capabilities(),
        token_parameter: OutputTokenParameter::MaxTokens,
        max_output_tokens: 100,
        // Latest managed fixtures include the production v2 text-tool schemas.
        // The synthetic test is about execution/admission, not the input limit.
        max_input_bytes: 32_000,
        limits: RunLimits {
            model_calls: Some(3),
            tool_calls: Some(3),
            output_bytes: Some(10000),
            tool_output_bytes: 10000,
            response: StreamLimits {
                text_bytes: 10000,
                argument_bytes: 10000,
                tool_calls: 3,
            },
        },
    }
}
fn confirmation(record: &SubmissionRecord, manuscript: Uuid) -> WritingExecutionConfirmation {
    WritingExecutionConfirmation {
        receipt: WritingSubmissionReceipt {
            conversation_id: record.conversation_id,
            request_id: record.request_id,
            manuscript_id: manuscript,
        },
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
    }
}
#[tokio::test]
async fn prepared_input_runs_and_duplicate_admission_never_replays_transport() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let prepared = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let record = prepared.record.clone();
    service
        .validate_start(
            &prepared,
            &confirmation(&record, f.manuscript),
            &profile(record.input.model.clone()),
        )
        .await
        .unwrap();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let transport = Arc::new(Transport(AtomicUsize::new(0)));
    let admitted = service
        .start_transport(
            sessions.clone(),
            record.clone(),
            profile(record.input.model.clone()),
            transport.clone(),
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!()
    };
    assert_eq!(
        owned.task.wait().await,
        TaskCompletion::Completed,
        "persisted failure: {}",
        persisted_failure(&sessions, &record, &f.store).await
    );
    let repeated = service
        .start_transport(
            sessions.clone(),
            record.clone(),
            profile(record.input.model.clone()),
            transport.clone(),
        )
        .await
        .unwrap();
    assert!(matches!(
        repeated,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Completed),
            ..
        }
    ));
    assert_eq!(transport.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        sessions
            .open(ConversationId::from_uuid(record.conversation_id))
            .await
            .unwrap()
            .turn(record.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Completed)
    );
}
#[tokio::test]
async fn stale_confirmation_and_unbound_input_do_not_start_a_turn() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let mut prepared = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let mut stale = confirmation(&prepared.record, f.manuscript);
    stale.context_snapshot_id = Uuid::new_v4();
    assert!(matches!(
        service
            .validate_start(
                &prepared,
                &stale,
                &profile(prepared.record.input.model.clone())
            )
            .await,
        Err(WritingStartError::Mismatch)
    ));
    prepared.record.input.text.push_str(" tampered");
    assert!(matches!(
        service
            .validate_start(
                &prepared,
                &confirmation(&prepared.record, f.manuscript),
                &profile(prepared.record.input.model.clone())
            )
            .await,
        Err(WritingStartError::Preparation(
            WritingSubmissionError::Input(SubmissionError::Conflict)
        ))
    ));
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(matches!(
        service
            .validate_start(
                &prepared,
                &confirmation(&prepared.record, f.manuscript),
                &profile(prepared.record.input.model.clone())
            )
            .await,
        Err(WritingStartError::Preparation(
            WritingSubmissionError::Unbound
        ))
    ));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_items", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn source_profile_never_grants_arbitrary_tools() {
    assert!(
        crate::agent::skills::tools::SkillTools::new(vec![], 4096)
            .unwrap()
            .validate(&ToolCall {
                namespace: None,
                raw_arguments_json: None,
                call_id: "x".into(),
                name: "shell".into(),
                arguments: serde_json::json!({})
            })
            .is_err()
    );
}

#[tokio::test]
async fn unknown_child_creation_never_starts_a_second_model_request() {
    struct SpawnTransport(AtomicUsize);
    impl ModelTransport for SpawnTransport {
        fn capabilities(&self) -> ModelCapabilities {
            Transport(AtomicUsize::new(0)).capabilities()
        }
        fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
            let index = self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if index != 0 {
                    let result = request
                        .messages
                        .iter()
                        .rev()
                        .find_map(|message| match message {
                            ModelMessage::ToolResult { content, .. } => Some(content.as_str()),
                            _ => None,
                        });
                    panic!("unknown creation must not sample again: {result:?}");
                }
                assert!(
                    request
                        .tools
                        .iter()
                        .any(|tool| tool.name == "research_spawn_subtask")
                );
                Ok(Box::new(Stream(
                    vec![
                        ModelStreamEvent::Started {
                            response_id: "child-fixture".into(),
                            actual_model: request.model,
                        },
                        ModelStreamEvent::ToolCallStarted {
                            call_id: "child-call".into(),
                            name: "research_spawn_subtask".into(),
                        },
                        ModelStreamEvent::ToolArgumentsDelta {
                            call_id: "child-call".into(),
                            delta: r#"{"prompt":"Compare approaches"}"#.into(),
                        },
                        ModelStreamEvent::ToolCallCompleted {
                            call_id: "child-call".into(),
                        },
                        ModelStreamEvent::Completed {
                            reason: FinishReason::ToolCalls,
                            end_turn: None,
                        },
                    ]
                    .into(),
                )) as Box<dyn ModelStream>)
            })
        }
    }
    let f = fixture(true);
    let service =
        WritingSubmissionService::from_app_store(&f.store).with_model_resolver(f.models.clone());
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    use research_agent_core::{conversation::ConversationCatalog, submission::SubmissionStore};
    let binding = research_agent_core::conversation::ConversationBinding {
        kind: research_agent_core::conversation::BindingKind::Manuscript,
        resource_id: f.manuscript.to_string(),
    };
    assert!(
        crate::agent::repository::SqliteConversationCatalog::from_app_store(&f.store)
            .read(ConversationId::from_uuid(f.conversation), Some(binding))
            .await
            .is_ok(),
        "parent binding"
    );
    assert!(
        crate::agent::repository::SqliteSubmissionStore::for_app(&f.store)
            .get_by_turn(f.conversation, record.turn_id)
            .await
            .is_ok(),
        "parent submission"
    );
    f.store.connection().unwrap().execute_batch("create table child_unknown_parent(id integer primary key);
      create table child_unknown_deferred(id integer references child_unknown_parent(id) deferrable initially deferred);
      create trigger child_unknown_commit after insert on data_sql_operation_receipts when new.kind='agent.conversation.create_bound'
      begin insert into child_unknown_deferred(id) values(9); end;").unwrap();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let transport = Arc::new(SpawnTransport(AtomicUsize::new(0)));
    let admission = service
        .start_transport_with_manuscript(
            sessions,
            record.clone(),
            profile(record.input.model.clone()),
            transport.clone(),
            f.manuscript,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    assert_eq!(transport.0.load(Ordering::SeqCst), 1);
    assert!(
        f.store.connection().is_err(),
        "the child commit fault must block its original data owner"
    );
}

#[tokio::test]
async fn compile_diagnostics_result_continues_without_receipt_barrier() {
    struct DiagnosticsTransport(AtomicUsize);
    impl ModelTransport for DiagnosticsTransport {
        fn capabilities(&self) -> ModelCapabilities {
            Transport(AtomicUsize::new(0)).capabilities()
        }
        fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
            let index = self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                let mut events = vec![ModelStreamEvent::Started {
                    response_id: "diagnostics-fixture".into(),
                    actual_model: request.model,
                }];
                if index == 0 {
                    assert!(
                        request
                            .tools
                            .iter()
                            .any(|tool| tool.name == "research_read_compile_diagnostics")
                    );
                    events.extend([
                        ModelStreamEvent::ToolCallStarted {
                            call_id: "diagnostics".into(),
                            name: "research_read_compile_diagnostics".into(),
                        },
                        ModelStreamEvent::ToolArgumentsDelta {
                            call_id: "diagnostics".into(),
                            delta: "{}".into(),
                        },
                        ModelStreamEvent::ToolCallCompleted {
                            call_id: "diagnostics".into(),
                        },
                        ModelStreamEvent::Completed {
                            reason: FinishReason::ToolCalls,
                            end_turn: None,
                        },
                    ]);
                } else {
                    assert_eq!(index, 1);
                    assert!(request.messages.iter().any(|message| matches!(message,
                        ModelMessage::ToolResult { content, is_error: false, .. } if content.contains("no_compile_run"))));
                    events.extend([
                        ModelStreamEvent::TextDelta("No compile yet".into()),
                        ModelStreamEvent::Completed {
                            reason: FinishReason::Stop,
                            end_turn: None,
                        },
                    ]);
                }
                Ok(Box::new(Stream(events.into())) as Box<dyn ModelStream>)
            })
        }
    }
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let prepared = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let record = prepared.record;
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let transport = Arc::new(DiagnosticsTransport(AtomicUsize::new(0)));
    let admission = service
        .start_transport(
            sessions.clone(),
            record.clone(),
            profile(record.input.model.clone()),
            transport.clone(),
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert_eq!(transport.0.load(Ordering::SeqCst), 2);
    let history = sessions
        .read_since(ConversationId::from_uuid(record.conversation_id), 0, 100)
        .await
        .unwrap();
    assert!(!history.items.iter().any(|event| matches!(
        event.payload(),
        research_agent_core::protocol::EventPayload::ToolReceiptPending { .. }
            | research_agent_core::protocol::EventPayload::ToolReceiptConfirmed { .. }
    )));
}

#[tokio::test]
async fn confirmed_chatgpt_run_uses_native_auth_port_and_persists_failure() {
    exercise_chatgpt_profile(None).await;
    exercise_chatgpt_profile(Some("medium")).await;
}

async fn exercise_chatgpt_profile(effort: Option<&str>) {
    struct Auth(AtomicUsize);
    impl crate::agent::authentication::ChatGptCredentialSource for Auth {
        fn acquire(&self) -> crate::agent::authentication::CredentialFuture<'_> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(crate::agent::authentication::AuthenticationError::Unavailable) })
        }
    }
    let mut f = fixture(true);
    // This exercises the production Responses state adapter, not the older
    // submission-only fixture schema. Upgrade before admitting the task.
    crate::storage::migrate(&mut f.store.connection().unwrap()).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update llm_provider_configs set enabled=1,base_url='' where provider_id='openai'",
            [],
        )
        .unwrap();
    let auth = Arc::new(Auth(AtomicUsize::new(0)));
    f.models = f.models.with_chatgpt(auth.clone());
    let service = WritingSubmissionService::from_app_store(&f.store);
    let mut input = request(&f, Uuid::new_v4());
    input.model = Some(AgentModelSelection {
        provider_id: "openai".into(),
        model_id: "gpt-5.6-terra".into(),
        reasoning_effort: effort.map(str::to_owned),
        service_tier: None,
    });
    let prepared = service.prepare(&f.models, input).await.unwrap();
    let record = prepared.record.clone();
    assert_eq!(auth.0.load(Ordering::SeqCst), 0);
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let mut execution_profile = profile(record.input.model.clone());
    if effort.is_some() {
        assert!(matches!(
            service
                .validate_start(
                    &prepared,
                    &confirmation(&record, f.manuscript),
                    &execution_profile
                )
                .await,
            Err(WritingStartError::Mismatch)
        ));
        assert_eq!(auth.0.load(Ordering::SeqCst), 0);
    }
    execution_profile.capabilities.reasoning_efforts =
        record.input.reasoning_effort.clone().into_iter().collect();
    let admitted = service
        .start_confirmed(
            sessions.clone(),
            prepared,
            confirmation(&record, f.manuscript),
            execution_profile,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admitted else {
        panic!("new run expected")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Failed);
    assert_eq!(
        auth.0.load(Ordering::SeqCst),
        1,
        "persisted failure: {}",
        persisted_failure(&sessions, &record, &f.store).await
    );
    assert_eq!(
        sessions
            .open(ConversationId::from_uuid(record.conversation_id))
            .await
            .unwrap()
            .turn(record.turn_id)
            .unwrap()
            .outcome,
        Some(TurnOutcome::Failed)
    );
}

#[path = "tests/runtime_sources.rs"]
mod runtime_sources;
