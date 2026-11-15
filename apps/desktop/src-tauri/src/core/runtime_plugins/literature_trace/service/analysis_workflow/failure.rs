//! One accepted original failure transaction, followed by its original history read.
use super::*;
use crate::{
    data_interface::guarded_transaction,
    hbs_storage::boundary::{BoundaryError, HbsOutcome},
    runtime_plugins::literature_analysis::{
        commands::original_error,
        failure_store::{self, FailureWrite},
        workflow_failure as errors,
        workflow_resources::WorkflowResources,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::analysis_history::{
    HistoryRequestV1, contracts::PaperAnalysisWorkflow,
};
use serde_json::Value;

pub(crate) struct AttemptFailure {
    pub workflow_id: String,
    pub turn_kind: String,
    pub error_code: String,
    pub error_message: String,
}

impl LiteratureTraceService {
    pub(crate) async fn fail_analysis_attempt(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: AttemptFailure,
    ) -> Result<PaperAnalysisWorkflow, Value> {
        let owner = Arc::clone(self);
        let workflow = request.workflow_id.clone();
        let selected = key.clone();
        tokio::spawn(async move { owner.fail_analysis_attempt_owned(store, key, request).await })
            .await
            .map_err(|_| {
                errors::annotate(
                    errors::native(
                        Self::error(
                            "plugin_host_source_worker_unsettled",
                            "论文分析失败保存未确认完成。",
                        ),
                        "execution",
                        HbsOutcome::Unknown,
                    ),
                    &workflow,
                    "execution",
                    HbsOutcome::Unknown,
                )
            })?
            .map_err(|error| errors::identify(error, &workflow, &selected))
    }

    async fn fail_analysis_attempt_owned(
        &self,
        store: AppStore,
        key: String,
        request: AttemptFailure,
    ) -> Result<PaperAnalysisWorkflow, Value> {
        self.available()
            .map_err(|error| errors::native(error, "admission", HbsOutcome::NotStarted))?;
        let _lifetime = self.doi_operations.read().await;
        self.available()
            .map_err(|error| errors::native(error, "admission", HbsOutcome::NotStarted))?;
        self.save_analysis_failure(store, key, request).await
    }

    async fn save_analysis_failure(
        &self,
        store: AppStore,
        key: String,
        request: AttemptFailure,
    ) -> Result<PaperAnalysisWorkflow, Value> {
        let (decision, origin) = self
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::ValidateTurn {
                    turn_kind: request.turn_kind.clone(),
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
        let source = store.clone();
        let workflow = request.workflow_id.clone();
        let kind = request.turn_kind.clone();
        let (resources, target) = tokio::task::spawn_blocking(move || {
            let package = package::accept_analysis_workflow(&catalog, &selected)
                .map_err(|error| errors::native(error, "resource", HbsOutcome::NotStarted))?;
            let resources = WorkflowResources::load(&catalog, &package).map_err(|error| {
                errors::native(
                    Self::error(error.code, error.message).package(&selected),
                    "resource",
                    HbsOutcome::NotStarted,
                )
            })?;
            let target = {
                let db = source.connection().map_err(|error| {
                    errors::original(error.into(), "source-read", HbsOutcome::NotStarted)
                })?;
                failure_store::capture(
                    &db,
                    resources.failure_statements().running,
                    &workflow,
                    &kind,
                )
                .map_err(|error| {
                    errors::original(original_error(error), "source-read", HbsOutcome::NotStarted)
                })?
            };
            Ok::<_, Value>((resources, target))
        })
        .await
        .map_err(|_| unsettled())??;
        let (decision, _origin) = self
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::FailureStatuses {
                    turn_kind: request.turn_kind.clone(),
                    error_code: request.error_code.clone(),
                },
            )
            .await
            .map_err(|error| errors::native(error, "validation", HbsOutcome::NotStarted))?;
        let WorkflowDecisionV1::Failure { statuses } = decision else {
            return Err(mismatch());
        };
        let workflow = request.workflow_id.clone();
        let source = store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = source.connection().map_err(|error| {
                errors::original(error.into(), "source-read", HbsOutcome::NotStarted)
            })?;
            guarded_transaction::run_mut(&mut db, |db| {
                let sql = resources.failure_statements();
                let current = match failure_store::capture(
                    db,
                    sql.running,
                    &request.workflow_id,
                    &request.turn_kind,
                ) {
                    Ok(current) => current,
                    Err(error) => {
                        return (
                            Err(errors::original(
                                original_error(error),
                                "source-read",
                                HbsOutcome::NotStarted,
                            )),
                            false,
                        );
                    }
                };
                if current != target {
                    return (
                        Err(errors::native(
                            Self::error(
                                "plugin_host_source_changed",
                                "论文分析运行中的尝试已变化，请重新载入。",
                            ),
                            "source-read",
                            HbsOutcome::NotStarted,
                        )),
                        false,
                    );
                }
                let result = failure_store::apply(
                    db,
                    &sql,
                    FailureWrite {
                        target: &target,
                        workflow_id: &request.workflow_id,
                        statuses: &statuses,
                        error_code: &request.error_code,
                        error_message: &request.error_message,
                    },
                    true,
                );
                let unknown = !db.is_autocommit()
                    || result.as_ref().is_err_and(|error| match error {
                        BoundaryError::Failure(failure) => failure.needs_recovery(),
                        BoundaryError::Pause(_) => false,
                    });
                (
                    result.map_err(|error| match error {
                        BoundaryError::Failure(error) => errors::boundary(error),
                        BoundaryError::Pause(pause) => match pause.need() {},
                    }),
                    unknown,
                )
            })
        })
        .await
        .map_err(|_| {
            errors::annotate(unsettled(), &workflow, "execution", HbsOutcome::Unknown)
        })??;
        let read = HistoryRequestV1::Get {
            workflow_id: workflow.clone(),
        };
        self.capture_project_analysis_history(store, key.clone(), read.clone())
            .await
            .map_err(|error| {
                errors::annotate(
                    error.into_wire(&read, Some(&key)),
                    &workflow,
                    "postcommit-read",
                    HbsOutcome::Committed,
                )
            })?
            .ok_or_else(|| {
                errors::annotate(
                    mismatch(),
                    &workflow,
                    "postcommit-read",
                    HbsOutcome::Committed,
                )
            })
    }
}

fn mismatch() -> Value {
    errors::native(
        LiteratureTraceService::error(
            "plugin_host_response_mismatch",
            "论文分析失败保存结果与本次输入不匹配。",
        ),
        "validation",
        HbsOutcome::NotStarted,
    )
}
fn unsettled() -> Value {
    errors::native(
        LiteratureTraceService::error(
            "plugin_host_source_worker_unsettled",
            "论文分析失败保存未确认完成。",
        ),
        "execution",
        HbsOutcome::Unknown,
    )
}
