//! Original attempt preparation and admission share the accepted service lifetime.
use super::{super::*, source, worker};
use crate::{
    ai::{
        analysis_support::{AnalysisSupportError, FrozenResources},
        error::{AiCommandError, AiError},
        paper_context::agent_preview,
        paper_reading_contracts::PaperContextRequest,
    },
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::literature_analysis::{
        attempt_resources::AttemptResources, resources::HistoryResources,
        workflow_failure as errors,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_attempt::{AnalysisAttemptTurnObservation, AnalysisAttemptWorkflowObservation},
    analysis_history::{
        HistoryRequestV1,
        contracts::{PaperAnalysisMessage, PaperAnalysisWorkflow},
    },
};
use serde_json::json;
use uuid::Uuid;

pub(crate) struct StartAnalysisAttempt {
    pub workflow_id: String,
    pub turn_kind: String,
    pub messages: Vec<PaperAnalysisMessage>,
    pub resources: FrozenResources,
}
impl LiteratureTraceService {
    pub(crate) async fn start_analysis_attempt(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: StartAnalysisAttempt,
    ) -> Result<PaperAnalysisWorkflow, AiCommandError> {
        let owner = Arc::clone(self);
        let workflow = request.workflow_id.clone();
        let selected = key.clone();
        tokio::spawn(async move { owner.start_attempt_owned(store, key, request).await })
            .await
            .map_err(|_| {
                worker::command(
                    worker::unknown("execution", "Core 分析尝试工作未确认完成。"),
                    &workflow,
                    None,
                    &selected,
                )
            })?
            .map_err(|error| worker::command(json!(error), &workflow, None, &selected))
    }
    async fn start_attempt_owned(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        mut request: StartAnalysisAttempt,
    ) -> Result<PaperAnalysisWorkflow, AiCommandError> {
        self.available()
            .map_err(super::super::template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        self.available()
            .map_err(super::super::template::technical)?;
        self.validate_analysis_attempt_input(&key, &request.turn_kind, &request.messages)
            .await?;
        let mut observed = self
            .capture_project_analysis_history_observed(
                store.clone(),
                key.clone(),
                HistoryRequestV1::Get {
                    workflow_id: request.workflow_id.clone(),
                },
            )
            .await
            .map_err(|error| error.into_ai(&key))?;
        let workflow = observed.workflow.take().ok_or_else(|| {
            super::super::template::identified(
                "plugin_host_response_mismatch",
                "分析尝试缺少本次工作流。",
                &observed.origin,
            )
        })?;
        let observation = AnalysisAttemptWorkflowObservation {
            status: workflow.status.clone(),
            turns: workflow
                .turns
                .iter()
                .map(|turn| AnalysisAttemptTurnObservation {
                    id: turn.id.clone(),
                    kind: turn.kind.clone(),
                    status: turn.status.clone(),
                    attempt_numbers: turn
                        .attempts
                        .iter()
                        .map(|attempt| attempt.attempt)
                        .collect(),
                })
                .collect(),
        };
        let (plan, _) = self
            .plan_analysis_attempt(&key, &request.turn_kind, observation)
            .await?;
        let attempt = Uuid::new_v4().to_string();
        let workflow_id = request.workflow_id.clone();
        let result = async {
            let resources = self
                .load_attempt_resources(&key, workflow.conversation_id.is_some())
                .await?;
            let facts = {
                let source = store.clone();
                let queries = Arc::clone(&resources);
                let workflow = workflow.clone();
                let messages = request.messages.clone();
                tokio::task::spawn_blocking(move || {
                    let db = source.connection()?;
                    source::capture(&db, &source, queries.context.as_ref(), &workflow, &messages)
                })
                .await
                .map_err(|_| AiError::TemporaryUnavailable)??
            };
            let shared = if let Some(facts) = facts {
                Some(
                    self.prepare_attempt_shared(
                        &store,
                        &key,
                        &workflow,
                        &attempt,
                        &mut request,
                        facts,
                    )
                    .await?,
                )
            } else {
                None
            };
            let (messages, messages_json) = tokio::task::spawn_blocking(move || {
                serde_json::to_string(&request.messages)
                    .map(|json| (request.messages, json))
                    .map_err(|_| AiError::TemporaryUnavailable)
            })
            .await
            .map_err(|_| AiError::TemporaryUnavailable)??;
            let (decision, origin) = self
                .analysis_workflow_decision(
                    &key,
                    WorkflowInputV1::RunningStatus {
                        turn_kind: request.turn_kind.clone(),
                    },
                )
                .await
                .map_err(super::super::template::technical)?;
            let WorkflowDecisionV1::Status { status } = decision else {
                return Err(AiCommandError::from(super::super::template::identified(
                    "plugin_host_response_mismatch",
                    "分析尝试运行状态种类不匹配。",
                    &origin,
                )));
            };
            self.available()
                .map_err(super::super::template::technical)?;
            worker::commit(
                store.clone(),
                resources,
                worker::PreparedAdmission {
                    workflow_id: request.workflow_id,
                    workflow,
                    observed,
                    plan,
                    attempt: attempt.clone(),
                    turn_kind: request.turn_kind,
                    messages,
                    messages_json,
                    status,
                    shared,
                },
            )
            .await?;
            let read = self
                .capture_project_analysis_history_observed(
                    store,
                    key.clone(),
                    HistoryRequestV1::Get {
                        workflow_id: workflow_id.clone(),
                    },
                )
                .await
                .map_err(|error| {
                    AiCommandError::from_analysis_workflow(errors::original(
                        error.into_ai(&key),
                        "postcommit-read",
                        HbsOutcome::Committed,
                    ))
                })?;
            read.workflow.ok_or_else(|| {
                AiCommandError::from_analysis_workflow(errors::original(
                    AiError::TemporaryUnavailable,
                    "postcommit-read",
                    HbsOutcome::Committed,
                ))
            })
        }
        .await;
        result.map_err(|error| worker::command(json!(error), &workflow_id, Some(&attempt), &key))
    }

    async fn load_attempt_resources(
        &self,
        key: &str,
        context: bool,
    ) -> Result<Arc<worker::Resources>, AiError> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)?;
            package::accept_analysis_history(&catalog, &selected)?;
            let map = |error: crate::runtime_plugins::RuntimePluginError| {
                LiteratureTraceService::error(error.code, error.message).package(&selected)
            };
            let attempt = AttemptResources::load(&catalog, &package).map_err(map)?;
            let history = HistoryResources::load(&catalog, &package).map_err(map)?;
            let context = if context {
                package::accept_paper_context(&catalog, &selected)?;
                Some(
                    crate::runtime_plugins::literature_context::PaperContextResources::load(
                        &catalog, &package,
                    )
                    .map_err(map)?,
                )
            } else {
                None
            };
            Ok::<_, Box<NativeHostError>>(Arc::new(worker::Resources {
                key: selected,
                attempt,
                history,
                context,
            }))
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
        .map_err(super::super::template::technical)
    }

    async fn prepare_attempt_shared(
        self: &Arc<Self>,
        store: &AppStore,
        key: &str,
        workflow: &PaperAnalysisWorkflow,
        attempt: &str,
        request: &mut StartAnalysisAttempt,
        facts: source::SharedFacts,
    ) -> Result<worker::Shared, AiError> {
        let (context, prompt, origin) = if let Some(input) = facts.prompt.clone() {
            let (prompt, origin) = self
                .render_analysis_task_prompt(key, input)
                .await
                .map_err(super::super::template::technical)?;
            let prompt =
                prompt.map_err(|error| super::super::template::rejected(error, &origin))?;
            (facts.grants.clone(), prompt, origin)
        } else {
            let preview = self
                .preview_analysis_context_in_operation(
                    store.clone(),
                    key.to_owned(),
                    PaperContextRequest {
                        paper_id: workflow.paper_id.clone(),
                        snapshot_id: facts.snapshot.id.clone(),
                        expected_context_hash: facts.snapshot.context_hash.clone(),
                    },
                )
                .await?;
            let context = agent_preview::submission_context_from_preview(preview)
                .map_err(|error| AnalysisSupportError::paper_context(error).into_ai_error())?;
            let (decision, origin) = self
                .analysis_workflow_decision(key, WorkflowInputV1::PackSources { sources: context })
                .await
                .map_err(super::super::template::technical)?;
            let WorkflowDecisionV1::Sources { result } = decision else {
                return Err(super::super::template::identified(
                    "plugin_host_response_mismatch",
                    "分析尝试上下文打包结果种类不匹配。",
                    &origin,
                ));
            };
            let context =
                result.map_err(|error| super::super::template::rejected(error, &origin))?;
            let (prompt, prompt_origin) = self
                .render_analysis_attempt_prompt(key, request.messages.clone())
                .await
                .map_err(super::super::template::technical)?;
            let prompt =
                prompt.map_err(|error| super::super::template::rejected(error, &prompt_origin))?;
            (context, prompt, origin)
        };
        let input = source::submission(
            workflow,
            attempt,
            &facts,
            context,
            prompt,
            std::mem::take(&mut request.resources),
        )?;
        Ok(worker::Shared {
            facts,
            input,
            origin,
        })
    }
}
