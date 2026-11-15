//! Original attempt failure transaction. Native supplies only the status values.
use super::error::AnalysisError;
use crate::{
    ai::{analysis_support::AnalysisAdmission, structured_resource_validation::Flow},
    hbs_storage::boundary::{self, BoundaryError, BoundaryFailure, HbsOutcome},
};
use research_workbench_native_sdk::analysis_workflow::FailureStatuses;
use rusqlite::{Connection, OptionalExtension, params};
use std::convert::Infallible;

pub(crate) struct FailureStatements<'a> {
    pub running: &'a str,
    pub attempt: &'a str,
    pub turn: &'a str,
    pub workflow: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunningAttempt {
    pub turn_id: String,
    pub attempt_id: String,
}

pub(crate) fn capture(
    db: &Connection,
    sql: &str,
    workflow: &str,
    kind: &str,
) -> Result<RunningAttempt, AnalysisError> {
    db.query_row(sql, params![workflow, kind], |row| {
        Ok(RunningAttempt {
            turn_id: row.get(0)?,
            attempt_id: row.get(1)?,
        })
    })
    .optional()?
    .ok_or_else(|| AnalysisError::InvalidInput("该论文分析阶段没有运行中的尝试".into()))
}

pub(crate) struct FailureWrite<'a> {
    pub target: &'a RunningAttempt,
    pub workflow_id: &'a str,
    pub statuses: &'a FailureStatuses,
    pub error_code: &'a str,
    pub error_message: &'a str,
}

pub(crate) fn apply(
    db: &mut Connection,
    sql: &FailureStatements<'_>,
    write: FailureWrite<'_>,
    explicit: bool,
) -> Result<(), BoundaryError<Infallible, AnalysisError>> {
    let transaction = db.transaction().map_err(|error| {
        BoundaryError::Failure(BoundaryFailure {
            failure: crate::ai::structured_resource_validation::Failure::original(error.into()),
            phase: "begin",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        })
    })?;
    let result = (|| -> Result<(), AnalysisError> {
        transaction.execute(
            sql.attempt,
            params![
                write.target.attempt_id,
                write.statuses.attempt,
                write.error_code,
                write.error_message
            ],
        )?;
        AnalysisAdmission::new(&transaction)
            .fail(&write.target.attempt_id, &write.statuses.attempt)?;
        transaction.execute(sql.turn, params![write.target.turn_id, write.statuses.turn])?;
        transaction.execute(
            sql.workflow,
            params![write.workflow_id, write.statuses.workflow],
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
