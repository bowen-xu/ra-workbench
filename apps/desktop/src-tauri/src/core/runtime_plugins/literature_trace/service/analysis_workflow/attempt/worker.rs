//! Final original read-set checks and fixed admission run on the Core guard.
use super::{super::*, source};
use crate::{
    ai::{analysis_support::AnalysisResources, error::AiCommandError},
    data_interface::guarded_transaction,
    hbs_storage::boundary::{BoundaryError, HbsOutcome},
    runtime_plugins::literature_analysis::{
        attempt_resources::AttemptResources,
        attempt_store::{self, StartAttemptWrite},
        capture,
        resources::HistoryResources,
        workflow_failure as errors,
    },
    storage::AppStore,
};
use research_agent_core::submission::SubmissionInput;
use research_workbench_native_sdk::{
    analysis_attempt::AnalysisAttemptPlan,
    analysis_history::{
        HistoryRequestV1,
        contracts::{PaperAnalysisMessage, PaperAnalysisWorkflow},
    },
};
use serde_json::{Value, json};
use uuid::Uuid;

pub(super) struct Resources {
    pub key: String,
    pub attempt: AttemptResources,
    pub history: HistoryResources,
    pub context: Option<crate::runtime_plugins::literature_context::PaperContextResources>,
}
pub(super) struct Shared {
    pub facts: source::SharedFacts,
    pub input: (Uuid, Uuid, SubmissionInput),
    pub origin: DecisionOrigin,
}
pub(super) struct PreparedAdmission {
    pub workflow_id: String,
    pub workflow: PaperAnalysisWorkflow,
    pub observed: super::super::super::analysis_history::ObservedHistory,
    pub plan: AnalysisAttemptPlan,
    pub attempt: String,
    pub turn_kind: String,
    pub messages: Vec<PaperAnalysisMessage>,
    pub messages_json: String,
    pub status: String,
    pub shared: Option<Shared>,
}

pub(super) async fn commit(
    store: AppStore,
    resources: Arc<Resources>,
    mut prepared: PreparedAdmission,
) -> Result<(), AiCommandError> {
    let workflow = prepared.workflow_id.clone();
    let attempt = prepared.attempt.clone();
    let key = resources.key.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(AiCommandError::from)?;
        let result = guarded_transaction::run_mut(&mut db, |db| {
            let result = (|| -> Result<(), Value> {
                if !db.is_autocommit() {
                    return Err(unknown("begin", "Core 分析尝试接纳前已有未结算事务。"));
                }
                let read = HistoryRequestV1::Get {
                    workflow_id: prepared.workflow_id.clone(),
                };
                let current = capture::capture(db, &read, &resources.history.statements())
                    .map_err(|error| {
                        errors::original(error, "source-read", HbsOutcome::NotStarted)
                    })?;
                if current != prepared.observed.input.workflow {
                    return Err(changed(
                        &prepared.observed.origin,
                        "论文分析工作流在尝试准备期间已变化。",
                    ));
                }
                let current = source::capture(
                    db,
                    &store,
                    resources.context.as_ref(),
                    &prepared.workflow,
                    &prepared.messages,
                )
                .map_err(|error| errors::original(error, "source-read", HbsOutcome::NotStarted))?;
                if current.as_ref() != prepared.shared.as_ref().map(|shared| &shared.facts) {
                    let origin = prepared
                        .shared
                        .as_ref()
                        .map_or(&prepared.observed.origin, |shared| &shared.origin);
                    return Err(changed(origin, "论文分析共享输入在尝试准备期间已变化。"));
                }
                let authority = AnalysisResources::new(&store);
                let shared = prepared.shared.take().map(|shared| shared.input);
                match attempt_store::apply_start(
                    db,
                    &resources.attempt.start(),
                    StartAttemptWrite {
                        workflow: &prepared.workflow,
                        workflow_id: &prepared.workflow_id,
                        plan: &prepared.plan,
                        attempt_id: &prepared.attempt,
                        turn_kind: &prepared.turn_kind,
                        messages_json: &prepared.messages_json,
                        workflow_status: &prepared.status,
                        shared,
                    },
                    Some(&authority),
                    true,
                ) {
                    Ok(()) => Ok(()),
                    Err(BoundaryError::Pause(pause)) => match pause.need() {},
                    Err(BoundaryError::Failure(failure)) => {
                        let unconfirmed = failure.needs_recovery();
                        let mut error = errors::boundary(failure);
                        if unconfirmed {
                            error["code"] = json!("data_operation_outcome_unknown");
                        }
                        Err(error)
                    }
                }
            })();
            let unconfirmed = !db.is_autocommit()
                || result
                    .as_ref()
                    .is_err_and(|error| error["analysisWorkflowOutcome"] == "unknown");
            (result, unconfirmed)
        });
        result.map_err(|error| {
            command(
                error,
                &prepared.workflow_id,
                Some(&prepared.attempt),
                &resources.key,
            )
        })
    })
    .await
    .map_err(|_| {
        command(
            unknown("execution", "Core 分析尝试接纳工作未确认完成。"),
            &workflow,
            Some(&attempt),
            &key,
        )
    })?
}

pub(super) fn command(
    mut error: Value,
    workflow: &str,
    attempt: Option<&str>,
    key: &str,
) -> AiCommandError {
    error["workflowId"] = json!(workflow);
    error["analysisWorkflowOperation"] = json!("startAttempt");
    if let Some(attempt) = attempt {
        error["attemptId"] = json!(attempt);
    }
    if error["nativeFailure"].is_object() && error["nativeFailure"]["packageKey"].is_null() {
        error["nativeFailure"]["packageKey"] = json!(key);
    }
    AiCommandError::from_analysis_workflow(error)
}
pub(super) fn unknown(phase: &'static str, message: &str) -> Value {
    let mut value = errors::native(
        LiteratureTraceService::error("plugin_host_source_worker_unsettled", message),
        phase,
        HbsOutcome::Unknown,
    );
    value["code"] = json!("data_operation_outcome_unknown");
    value["analysisWorkflowPhase"] = json!(phase);
    value["analysisWorkflowOutcome"] = json!(HbsOutcome::Unknown);
    value
}
fn changed(origin: &DecisionOrigin, message: &str) -> Value {
    errors::original(
        super::super::template::identified("plugin_host_source_changed", message, origin),
        "source-read",
        HbsOutcome::NotStarted,
    )
}
