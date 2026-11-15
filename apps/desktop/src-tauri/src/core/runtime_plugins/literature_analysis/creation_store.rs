//! Original workflow creation transaction, including Core shared-conversation binding.
use super::error::AnalysisError;
use crate::{
    ai::{
        analysis_support::AnalysisAdmission,
        structured_resource_validation::{Failure, Flow},
    },
    hbs_storage::boundary::{self, BoundaryError, BoundaryFailure, HbsOutcome},
};
use research_workbench_native_sdk::analysis_workflow::PaperAnalysisWorkflowDraft;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

pub(crate) struct CreationStatements<'a> {
    pub latest: &'a str,
    pub supersede: &'a str,
    pub create: &'a str,
    pub turn: &'a str,
}
pub(crate) struct CreationIdentity {
    pub workflow_id: String,
    turn_id: String,
}
impl CreationIdentity {
    pub(crate) fn new() -> Self {
        Self {
            workflow_id: Uuid::new_v4().to_string(),
            turn_id: Uuid::new_v4().to_string(),
        }
    }
}
pub(crate) fn latest(
    db: &Connection,
    sql: &str,
    paper: &str,
) -> Result<Option<String>, AnalysisError> {
    db.query_row(sql, [paper], |row| row.get(0))
        .optional()
        .map_err(Into::into)
}

pub(crate) fn apply<P>(
    db: &mut Connection,
    sql: &CreationStatements<'_>,
    draft: &PaperAnalysisWorkflowDraft,
    supersedes: Option<&str>,
    identity: &CreationIdentity,
    explicit: bool,
    conversation: impl FnOnce(&Transaction<'_>, Uuid) -> Result<Uuid, Flow<AnalysisError, P>>,
) -> Result<String, BoundaryError<P, AnalysisError>> {
    let workflow_id = &identity.workflow_id;
    let overview_turn_id = &identity.turn_id;
    let transaction = db.transaction().map_err(|error| {
        BoundaryError::Failure(BoundaryFailure {
            failure: Failure::original(error.into()),
            phase: "begin",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        })
    })?;
    let result = conversation(&transaction, Uuid::new_v4()).and_then(|conversation_id| {
        (|| -> Result<String, AnalysisError> {
            if let Some(prior_id) = supersedes {
                transaction.execute(sql.supersede, [prior_id])?;
            }
            transaction.execute(
                sql.create,
                params![
                    workflow_id,
                    draft.paper_id,
                    draft.context_snapshot_id,
                    draft.provider_id,
                    draft.model_id,
                    draft.reasoning_effort,
                    draft.prompt_set_version,
                    supersedes
                ],
            )?;
            AnalysisAdmission::new(&transaction).bind_workflow(
                workflow_id,
                conversation_id,
                &draft.paper_id,
                &draft.context_snapshot_id,
            )?;
            transaction.execute(
                sql.turn,
                params![overview_turn_id, workflow_id, 0_i64, "paper_overview"],
            )?;
            Ok(workflow_id.clone())
        })()
        .map_err(Flow::original)
    });
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
