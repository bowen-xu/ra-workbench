//! Re-run the original transaction prefix after each confirmed Native pause.
use super::*;
use crate::{
    data_interface::guarded_transaction,
    hbs_storage::boundary::BoundaryError,
    runtime_plugins::literature_analysis::{
        commands::original_error,
        creation_policy::{CreationNeed, CreationPolicy},
        creation_store::{self, CreationIdentity},
    },
};

pub(super) enum Step {
    Created(String),
    Need(CreationNeed, HbsOutcome),
}

pub(super) async fn attempt(
    store: AppStore,
    resources: Arc<CreationResources>,
    draft: Arc<PaperAnalysisWorkflowDraft>,
    policy: CreationPolicy,
) -> Result<(CreationPolicy, Step), Value> {
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|error| {
            errors::original(error.into(), "source-read", HbsOutcome::NotStarted)
        })?;
        let result = guarded_transaction::run_mut(&mut db, |db| {
            if !db.is_autocommit() {
                return (
                    Err(errors::native(
                        LiteratureTraceService::error(
                            "plugin_host_source_worker_unsettled",
                            "论文分析创建连接尚有未确认事务。",
                        ),
                        "execution",
                        HbsOutcome::Unknown,
                    )),
                    true,
                );
            }
            let sql = resources.creation();
            let supersedes = match creation_store::latest(db, sql.latest, &draft.paper_id) {
                Ok(value) => value,
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
            let identity = CreationIdentity::new();
            let result = creation_store::apply(
                db,
                &sql,
                &draft,
                supersedes.as_deref(),
                &identity,
                true,
                |transaction, candidate| {
                    crate::runtime_plugins::literature_analysis::conversation_store::resolve_generation_conversation(
                        transaction, resources.conversation(), &policy, &draft.paper_id, candidate,
                    )
                },
            );
            let unknown = !db.is_autocommit()
                || result.as_ref().is_err_and(|error| match error {
                    BoundaryError::Failure(failure) => failure.needs_recovery(),
                    BoundaryError::Pause(_) => false,
                });
            let result = match result {
                Ok(id) => Ok(Step::Created(id)),
                Err(BoundaryError::Pause(pause)) => {
                    let outcome = pause.outcome();
                    Ok(Step::Need(pause.need(), outcome))
                }
                Err(BoundaryError::Failure(failure)) => {
                    let mut error = errors::boundary(failure);
                    error["workflowId"] = serde_json::json!(identity.workflow_id);
                    Err(error)
                }
            };
            (result, unknown)
        });
        result.map(|step| (policy, step))
    })
    .await
    .map_err(|_| {
        errors::native(
            LiteratureTraceService::error(
                "plugin_host_source_worker_unsettled",
                "论文分析创建事务未确认完成。",
            ),
            "execution",
            HbsOutcome::Unknown,
        )
    })?
}
