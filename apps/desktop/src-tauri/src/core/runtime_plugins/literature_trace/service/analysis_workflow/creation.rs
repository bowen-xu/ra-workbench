//! Original creation uses the same Core transaction and immutable Native decisions.
use super::*;
use crate::{
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::literature_analysis::{
        creation_policy::CreationPolicy, creation_resources::CreationResources,
        workflow_failure as errors,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_history::{HistoryRequestV1, contracts::PaperAnalysisWorkflow},
    analysis_workflow::PaperAnalysisWorkflowDraft,
};
use serde_json::{Value, json};
mod worker;

impl LiteratureTraceService {
    pub(crate) async fn create_analysis_workflow(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: PaperAnalysisWorkflowDraft,
    ) -> Result<PaperAnalysisWorkflow, Value> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let paper = draft.paper_id.clone();
        let snapshot = draft.context_snapshot_id.clone();
        let result = tokio::spawn(async move {
            owner
                .create_analysis_workflow_owned(store, key, draft)
                .await
        })
        .await
        .map_err(|_| {
            errors::native(
                Self::error(
                    "plugin_host_source_worker_unsettled",
                    "论文分析创建未确认完成。",
                ),
                "execution",
                HbsOutcome::Unknown,
            )
        })
        .and_then(|result| result);
        result.map_err(|mut error| {
            error["analysisWorkflowOperation"] = json!("createWorkflow");
            error["paperId"] = json!(paper);
            error["contextSnapshotId"] = json!(snapshot);
            if error["nativeFailure"].is_object() && error["nativeFailure"]["packageKey"].is_null()
            {
                error["nativeFailure"]["packageKey"] = json!(selected);
            }
            error
        })
    }

    async fn create_analysis_workflow_owned(
        &self,
        store: AppStore,
        key: String,
        draft: PaperAnalysisWorkflowDraft,
    ) -> Result<PaperAnalysisWorkflow, Value> {
        self.available()
            .map_err(|error| errors::native(error, "admission", HbsOutcome::NotStarted))?;
        let _lifetime = self.doi_operations.read().await;
        let (decision, origin) = self
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::ValidateDraft {
                    draft: draft.clone(),
                },
            )
            .await
            .map_err(|error| errors::native(error, "validation", HbsOutcome::NotStarted))?;
        match decision {
            WorkflowDecisionV1::Validated { rejection: None } => {}
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => return Err(errors::rejected(error.to_string(), origin, "validation")),
            _ => return Err(mismatch()),
        }
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let resources = Arc::new(
            tokio::task::spawn_blocking(move || {
                let package = package::accept_analysis_workflow(&catalog, &selected)
                    .map_err(|error| errors::native(error, "resource", HbsOutcome::NotStarted))?;
                CreationResources::load(&catalog, &package).map_err(|error| {
                    errors::native(
                        Self::error(error.code, error.message).package(&selected),
                        "resource",
                        HbsOutcome::NotStarted,
                    )
                })
            })
            .await
            .map_err(|_| {
                errors::native(
                    Self::error(
                        "plugin_host_source_worker_unsettled",
                        "论文分析创建资源准备未确认完成。",
                    ),
                    "resource",
                    HbsOutcome::NotStarted,
                )
            })??,
        );
        let mut policy = CreationPolicy::default();
        let draft = Arc::new(draft);
        let id = loop {
            let (next, step) = worker::attempt(
                store.clone(),
                Arc::clone(&resources),
                Arc::clone(&draft),
                policy,
            )
            .await?;
            policy = next;
            match step {
                worker::Step::Created(id) => break id,
                worker::Step::Need(need, outcome) => {
                    let Some(input) = need.input().cloned() else {
                        return Err(errors::native(
                            Self::error(
                                "plugin_host_source_changed",
                                "论文分析创建时的会话状态已变化，请重新载入。",
                            ),
                            "source-read",
                            outcome,
                        ));
                    };
                    let (decision, origin) = self
                        .analysis_workflow_decision(&key, input)
                        .await
                        .map_err(|error| errors::native(error, "validation", outcome))?;
                    if !policy.resolve(need, decision, origin) {
                        return Err(mismatch());
                    }
                }
            }
        };
        let request = HistoryRequestV1::Get {
            workflow_id: id.clone(),
        };
        self.capture_project_analysis_history(store, key.clone(), request.clone())
            .await
            .map_err(|error| {
                errors::annotate(
                    error.into_wire(&request, Some(&key)),
                    &id,
                    "postcommit-read",
                    HbsOutcome::Committed,
                )
            })?
            .ok_or_else(|| {
                errors::annotate(mismatch(), &id, "postcommit-read", HbsOutcome::Committed)
            })
    }
}

fn mismatch() -> Value {
    errors::native(
        LiteratureTraceService::error(
            "plugin_host_response_mismatch",
            "论文分析创建结果与本次输入不匹配。",
        ),
        "validation",
        HbsOutcome::NotStarted,
    )
}
