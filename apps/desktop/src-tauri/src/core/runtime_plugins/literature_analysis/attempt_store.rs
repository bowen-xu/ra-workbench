//! Original attempt admission transaction. Domain preparation completes before entry.
use super::error::AnalysisError;
use crate::{
    ai::{
        analysis_support::{AnalysisAdmission, AnalysisResources, AnalysisSubmission},
        structured_resource_validation::{Failure, Flow},
    },
    hbs_storage::boundary::{self, BoundaryError, BoundaryFailure, HbsOutcome},
};
use research_agent_core::submission::SubmissionInput;
use research_workbench_native_sdk::{
    analysis_attempt::AnalysisAttemptPlan, analysis_history::contracts::PaperAnalysisWorkflow,
};
use rusqlite::{Connection, params};
use std::convert::Infallible;
use uuid::Uuid;

pub(crate) struct StartAttemptStatements<'a> {
    pub attempt: &'a str,
    pub turn: &'a str,
    pub workflow: &'a str,
}
pub(crate) struct StartAttemptWrite<'a> {
    pub workflow: &'a PaperAnalysisWorkflow,
    pub workflow_id: &'a str,
    pub plan: &'a AnalysisAttemptPlan,
    pub attempt_id: &'a str,
    pub turn_kind: &'a str,
    pub messages_json: &'a str,
    pub workflow_status: &'a str,
    pub shared: Option<(Uuid, Uuid, SubmissionInput)>,
}
pub(crate) fn apply_start(
    db: &mut Connection,
    sql: &StartAttemptStatements<'_>,
    write: StartAttemptWrite<'_>,
    authority: Option<&AnalysisResources<'_>>,
    explicit: bool,
) -> Result<(), BoundaryError<Infallible, AnalysisError>> {
    let transaction = db.transaction().map_err(|error| {
        BoundaryError::Failure(BoundaryFailure {
            failure: Failure::original(error.into()),
            phase: "begin",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        })
    })?;
    let result = (|| -> Result<(), AnalysisError> {
        transaction.execute(
            sql.attempt,
            params![
                write.attempt_id,
                write.plan.turn_id,
                write.plan.next_attempt,
                write.messages_json,
                write.workflow.provider_id,
                write.workflow.model_id,
                write.workflow.reasoning_effort
            ],
        )?;
        if let Some((conversation_id, request_id, input)) = write.shared {
            AnalysisAdmission::new(&transaction).admit(
                authority,
                AnalysisSubmission {
                    attempt: write.attempt_id,
                    conversation: conversation_id,
                    request: request_id,
                    input,
                    paper: &write.workflow.paper_id,
                    snapshot: &write.workflow.context_snapshot_id,
                    task_kind: "structuredReading",
                },
            )?;
        }
        transaction.execute(sql.turn, [&write.plan.turn_id])?;
        transaction.execute(
            sql.workflow,
            params![write.workflow_id, write.workflow_status, write.turn_kind],
        )?;
        Ok(())
    })()
    .map_err(Flow::original);
    let settled = boundary::settle_with(
        transaction,
        result,
        "write",
        explicit,
        true,
        AnalysisError::from,
        |_| AnalysisError::TemporaryUnavailable,
    );
    boundary::confirm_with(db, settled, explicit, |_| {
        AnalysisError::TemporaryUnavailable
    })
}

pub(crate) struct CompleteAttemptStatements<'a> {
    pub attempt: &'a str,
    pub turn: &'a str,
    pub workflow: &'a str,
}
pub(crate) struct CompleteAttemptWrite<'a> {
    pub workflow_id: &'a str,
    pub turn_id: &'a str,
    pub attempt_id: &'a str,
    pub success: crate::ai::paper_analysis_contracts::AttemptSuccess<'a>,
    pub workflow_status: &'a str,
}
pub(crate) fn apply_complete(
    db: &mut Connection,
    sql: &CompleteAttemptStatements<'_>,
    write: CompleteAttemptWrite<'_>,
    explicit: bool,
) -> Result<(), BoundaryError<Infallible, AnalysisError>> {
    let transaction = db.transaction().map_err(|error| {
        BoundaryError::Failure(BoundaryFailure {
            failure: Failure::original(error.into()),
            phase: "begin",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        })
    })?;
    let result = (|| -> Result<(), AnalysisError> {
        let success = write.success;
        transaction.execute(
            sql.attempt,
            params![
                write.attempt_id,
                success.ai_run_id,
                success.candidate_id,
                success.input_hash,
                success.output_hash,
                success.input_tokens,
                success.output_tokens
            ],
        )?;
        AnalysisAdmission::new(&transaction).complete(
            write.attempt_id,
            success.ai_run_id,
            success.candidate_id,
        )?;
        transaction.execute(sql.turn, params![write.turn_id, success.candidate_id])?;
        transaction.execute(
            sql.workflow,
            params![write.workflow_id, write.workflow_status],
        )?;
        Ok(())
    })()
    .map_err(Flow::original);
    let settled = boundary::settle_with(
        transaction,
        result,
        "write",
        explicit,
        true,
        AnalysisError::from,
        |_| AnalysisError::TemporaryUnavailable,
    );
    boundary::confirm_with(db, settled, explicit, |_| {
        AnalysisError::TemporaryUnavailable
    })
}
