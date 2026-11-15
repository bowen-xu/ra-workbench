//! Original completion reads and writes stay on the Core connection owner.
use super::super::*;
use crate::{
    ai::{
        analysis_support::{AnalysisJournal, CandidateOutcome},
        error::AiError,
        paper_analysis_contracts::AttemptSuccess,
    },
    data_interface::guarded_transaction,
    hbs_storage::boundary::{BoundaryError, HbsOutcome},
    runtime_plugins::literature_analysis::{
        attempt_resources::AttemptResources,
        attempt_store::{self, CompleteAttemptWrite},
        commands::original_error,
        failure_store::{self, RunningAttempt},
        workflow_failure as errors,
        workflow_resources::WorkflowResources,
    },
    storage::AppStore,
};
use rusqlite::Connection;
use serde_json::Value;

pub(super) struct Resources {
    pub attempt: AttemptResources,
    pub workflow: WorkflowResources,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Observed {
    pub target: RunningAttempt,
    pub outcome: CandidateOutcome,
    pub has_concept_turn: bool,
}

pub(super) fn capture(
    db: &Connection,
    sql: &Resources,
    workflow: &str,
    run: &str,
) -> Result<Observed, AiError> {
    let running = || {
        failure_store::capture(
            db,
            sql.workflow.failure_statements().running,
            workflow,
            "paper_overview",
        )
        .map_err(original_error)
    };
    let target = running()?;
    let journal = AnalysisJournal::new(db);
    journal
        .ensure_completed(&target.attempt_id)
        .map_err(|error| error.into_ai_error())?;
    let outcome = journal
        .candidate_outcome(run, "paper_overview_unit_v1")
        .map_err(|error| error.into_ai_error())?;
    if !journal
        .candidate_matches(run, &outcome.candidate_id)
        .map_err(|error| error.into_ai_error())?
    {
        return Err(AiError::InvalidInput(
            "论文分析候选与 provider 运行不匹配".into(),
        ));
    }
    // Keep the second original running-attempt read after candidate validation.
    let target = running()?;
    let has_concept_turn = db.query_row(&sql.attempt.concept, [workflow], |row| row.get(0))?;
    Ok(Observed {
        target,
        outcome,
        has_concept_turn,
    })
}

pub(super) async fn commit(
    store: AppStore,
    resources: Resources,
    workflow: String,
    run: String,
    observed: Observed,
    status: String,
    origin: DecisionOrigin,
) -> Result<(), Value> {
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|error| {
            errors::original(error.into(), "source-read", HbsOutcome::NotStarted)
        })?;
        guarded_transaction::run_mut(&mut db, |db| {
            let result = (|| {
                if !db.is_autocommit() {
                    return Err(super::worker::unknown(
                        "begin",
                        "Core 分析尝试完成前已有未结算事务。",
                    ));
                }
                let current = capture(db, &resources, &workflow, &run).map_err(|error| {
                    errors::original(error, "source-read", HbsOutcome::NotStarted)
                })?;
                if current != observed {
                    return Err(errors::original(
                        super::super::template::identified(
                            "plugin_host_source_changed",
                            "论文分析完成来源在准备期间已变化，请重新载入。",
                            &origin,
                        ),
                        "source-read",
                        HbsOutcome::NotStarted,
                    ));
                }
                let outcome = &observed.outcome;
                attempt_store::apply_complete(
                    db,
                    &resources.attempt.complete(),
                    CompleteAttemptWrite {
                        workflow_id: &workflow,
                        turn_id: &observed.target.turn_id,
                        attempt_id: &observed.target.attempt_id,
                        success: AttemptSuccess {
                            ai_run_id: &run,
                            candidate_id: &outcome.candidate_id,
                            input_hash: Some(&outcome.input_hash),
                            output_hash: Some(&outcome.output_hash),
                            input_tokens: outcome.input_tokens,
                            output_tokens: outcome.output_tokens,
                        },
                        workflow_status: &status,
                    },
                    true,
                )
                .map_err(|error| match error {
                    BoundaryError::Pause(pause) => match pause.need() {},
                    BoundaryError::Failure(failure) => errors::boundary(failure),
                })
            })();
            let unconfirmed = !db.is_autocommit()
                || result
                    .as_ref()
                    .is_err_and(|error| error["analysisWorkflowOutcome"] == "unknown");
            (result, unconfirmed)
        })
    })
    .await
    .map_err(|_| super::worker::unknown("execution", "Core 分析尝试完成工作未确认完成。"))?
}
