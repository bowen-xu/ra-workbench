//! Complete the original overview attempt after the shared Agent has durably closed.
use super::{super::*, completion_store};
use crate::{
    ai::error::{AiCommandError, AiError},
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::literature_analysis::{
        attempt_resources::AttemptResources, workflow_failure as errors,
        workflow_resources::WorkflowResources,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::analysis_history::{
    HistoryRequestV1, contracts::PaperAnalysisWorkflow,
};
use serde_json::{Value, json};

impl LiteratureTraceService {
    pub(crate) async fn complete_analysis_overview(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        workflow: String,
        run: String,
    ) -> Result<PaperAnalysisWorkflow, AiCommandError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let id = workflow.clone();
        let run_id = run.clone();
        tokio::spawn(async move {
            owner
                .complete_analysis_overview_owned(store, key, workflow, run)
                .await
        })
        .await
        .map_err(|_| {
            command(
                super::worker::unknown("execution", "Core 分析尝试完成未确认结束。"),
                &id,
                &run_id,
                &selected,
            )
        })?
        .map_err(|error| command(error, &id, &run_id, &selected))
    }

    async fn complete_analysis_overview_owned(
        &self,
        store: AppStore,
        key: String,
        workflow: String,
        run: String,
    ) -> Result<PaperAnalysisWorkflow, Value> {
        self.available()
            .map_err(|error| errors::native(error, "admission", HbsOutcome::NotStarted))?;
        let _lifetime = self.doi_operations.read().await;
        self.available()
            .map_err(|error| errors::native(error, "admission", HbsOutcome::NotStarted))?;
        let source = store.clone();
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let id = workflow.clone();
        let run_id = run.clone();
        let (resources, observed) = tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)
                .map_err(|error| errors::native(error, "resource", HbsOutcome::NotStarted))?;
            let load_error = |error: crate::runtime_plugins::RuntimePluginError| {
                errors::native(
                    LiteratureTraceService::error(error.code, error.message).package(&selected),
                    "resource",
                    HbsOutcome::NotStarted,
                )
            };
            let resources = completion_store::Resources {
                attempt: AttemptResources::load(&catalog, &package).map_err(load_error)?,
                workflow: WorkflowResources::load(&catalog, &package).map_err(load_error)?,
            };
            let db = source.connection().map_err(|error| {
                errors::original(error.into(), "source-read", HbsOutcome::NotStarted)
            })?;
            let observed = completion_store::capture(&db, &resources, &id, &run_id)
                .map_err(|error| errors::original(error, "source-read", HbsOutcome::NotStarted))?;
            Ok::<_, Value>((resources, observed))
        })
        .await
        .map_err(|_| super::worker::unknown("source-read", "分析完成来源读取未确认结束。"))??;
        let (decision, origin) = self
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::CompletionStatus {
                    turn_kind: "paper_overview".into(),
                    has_concept_turn: observed.has_concept_turn,
                },
            )
            .await
            .map_err(|error| errors::native(error, "validation", HbsOutcome::NotStarted))?;
        let WorkflowDecisionV1::Status { status } = decision else {
            return Err(errors::original(
                super::super::template::identified(
                    "plugin_host_response_mismatch",
                    "分析尝试完成状态种类不匹配。",
                    &origin,
                ),
                "validation",
                HbsOutcome::NotStarted,
            ));
        };
        self.available()
            .map_err(|error| errors::native(error, "admission", HbsOutcome::NotStarted))?;
        let attempt_id = observed.target.attempt_id.clone();
        completion_store::commit(
            store.clone(),
            resources,
            workflow.clone(),
            run,
            observed,
            status,
            origin,
        )
        .await
        .map_err(|mut error| {
            error["attemptId"] = json!(attempt_id);
            error
        })?;
        let read = self
            .capture_project_analysis_history_observed(
                store,
                key.clone(),
                HistoryRequestV1::Get {
                    workflow_id: workflow,
                },
            )
            .await
            .map_err(|error| {
                errors::original(
                    error.into_ai(&key),
                    "postcommit-read",
                    HbsOutcome::Committed,
                )
            })?;
        read.workflow.ok_or_else(|| {
            errors::original(
                AiError::TemporaryUnavailable,
                "postcommit-read",
                HbsOutcome::Committed,
            )
        })
    }
}

fn command(mut error: Value, workflow: &str, run: &str, key: &str) -> AiCommandError {
    error = errors::identify(error, workflow, key);
    error["analysisWorkflowOperation"] = json!("completeOverview");
    error["runId"] = json!(run);
    if error["analysisWorkflowOutcome"] == "unknown" {
        error["code"] = json!("data_operation_outcome_unknown");
    }
    AiCommandError::from_analysis_workflow(error)
}
