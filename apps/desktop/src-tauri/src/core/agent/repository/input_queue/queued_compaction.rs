//! A queued command keeps I until the original Compact task has a terminal fact.
//! The data facade owns admission and the transaction; this is no task owner.
use super::*;
use crate::{agent::repository::submissions, data_interface::SqlMutationError};
use research_agent_core::{
    conversation::CatalogError, protocol::TurnOutcome, submission::SubmissionRecord,
};
use rusqlite::Transaction;
#[path = "queued_compaction/types.rs"]
mod types;
use types::PrepareRecord;
pub(crate) use types::{
    PrepareIntent, PrepareSource, QueuedCompactionRead, QueuedCompactionState, ReadIntent,
    SettleIntent, SettleState, prepare_command, settle_command,
};
#[path = "queued_compaction/read.rs"]
mod read;
pub(crate) use read::{read_in, require_startable_in};
#[path = "queued_compaction/prepare.rs"]
mod prepare;
pub(crate) use prepare::{prepare_in, prepare_with_context_in, replay_in};
#[path = "queued_compaction/settle.rs"]
mod settle;
pub(crate) use settle::settle_in;

/// Status checks immutable preparation, independently of today's live binding.
/// Mutation separately authorizes the current manuscript under its transaction.
pub(crate) fn frozen_scope_in(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    input: Uuid,
    request: Uuid,
) -> Result<bool, SqlMutationError> {
    let Some((claim, _)) = read::preparation(db, conversation, input, request)? else {
        return Ok(false);
    };
    let record = saved(db, conversation, request)?;
    crate::agent::writing_submission::validate_scope(&record, manuscript).map_err(writing)?;
    let expected =
        crate::agent::writing_submission::read_compaction_revision_in(db, conversation, request)
            .map_err(writing)?;
    if claim.manuscript_id != manuscript
        || expected != Some(claim.expected_revision)
        || record.input.text.trim() != "/compact"
    {
        return Err(CatalogError::Corrupt.into());
    }
    let origin: Option<String> = db.query_row(
        "select input_id from agent_queue_submission_origins where conversation_id=?1 and request_id=?2",
        params![conversation.to_string(), request.to_string()], |row| row.get(0),
    ).optional()?;
    Ok(origin == Some(input.to_string()))
}

/// Cancellation has its own typed SQL proof. A task must never be released by
/// this path, including a started task hidden by a later history rollback.
pub(crate) fn cancel_unstarted_in(
    tx: &Transaction<'_>,
    conversation: Uuid,
    manuscript: Uuid,
    request: Uuid,
    revision: u64,
    input: Uuid,
    preview: Uuid,
) -> Result<InputQueue, SqlMutationError> {
    require_open(tx, conversation).map_err(domain)?;
    require_startable_in(tx, conversation, preview)?;
    let record = saved(tx, conversation, preview)?;
    authorize(tx, conversation, manuscript)?;
    // System reconciliation may finish under its own request envelope. Its
    // valid terminal is already sufficient to reject unstarted cancellation.
    if read::terminal(tx, &record)?.is_some() {
        return Err(CatalogError::Conflict.into());
    }
    use submissions::start_observation::{self, Error};
    let observed = start_observation::observe(
        tx,
        &record,
        research_agent_core::protocol::TaskKind::Compact,
    )
    .map_err(|cause| match cause {
        Error::Unavailable(error) => SqlMutationError::Sql(error),
        _ => CatalogError::Corrupt.into(),
    })?;
    if observed.is_some() {
        return Err(CatalogError::Conflict.into());
    }
    super::cancel_preview::release_preview_in(tx, conversation, request, revision, input, preview)
}

fn domain(error: Error) -> SqlMutationError {
    match error {
        Error::Queue(QueueError::Invalid) => CatalogError::InvalidInput,
        Error::Queue(QueueError::Conflict) => CatalogError::Conflict,
        Error::Queue(QueueError::Busy | QueueError::Paused) => CatalogError::Busy,
        Error::Queue(QueueError::Capacity) => CatalogError::TooLarge,
        Error::Closed => CatalogError::NotFound,
        Error::Corrupt => CatalogError::Corrupt,
        Error::Storage => CatalogError::Unavailable,
    }
    .into()
}
fn writing(error: crate::agent::writing_submission::WritingSubmissionError) -> SqlMutationError {
    use crate::agent::writing_submission::WritingSubmissionError as E;
    use research_agent_core::submission::SubmissionError as S;
    match error {
        E::Unbound => CatalogError::NotFound,
        E::Input(S::Conflict) => CatalogError::Conflict,
        E::Input(S::InvalidInput) => CatalogError::InvalidInput,
        E::Input(S::Corrupt) => CatalogError::Corrupt,
        _ => CatalogError::Unavailable,
    }
    .into()
}
fn authorize(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
) -> Result<(), SqlMutationError> {
    crate::agent::writing_submission::check_binding(db, conversation, manuscript).map_err(writing)
}
fn queue(db: &Connection, conversation: Uuid) -> Result<InputQueue, SqlMutationError> {
    load(db, conversation).map_err(domain)
}
fn saved(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<SubmissionRecord, SqlMutationError> {
    submissions::read(db, conversation, request)
        .map_err(|error| domain(admission::submission_error(error)))?
        .ok_or(CatalogError::Corrupt.into())
}
fn save_queue(tx: &Transaction<'_>, queue: &InputQueue) -> Result<String, SqlMutationError> {
    let json = Snapshot::encode(queue).map_err(domain)?;
    let changed = tx.execute(
        "update agent_input_queues set revision=?2,snapshot_json=?3 where conversation_id=?1",
        params![queue.conversation_id().to_string(), queue.revision(), json],
    )?;
    if changed != 1 {
        return Err(CatalogError::Corrupt.into());
    }
    Ok(json)
}
fn current_request(
    db: &Connection,
    conversation: Uuid,
    input: Uuid,
) -> Result<Option<Uuid>, SqlMutationError> {
    let raw: Option<(String, String)> = db
        .query_row(
            "select conversation_id,request_id from agent_input_queue_previews where input_id=?1",
            [input.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    raw.map(|(scope, request)| {
        if scope != conversation.to_string() {
            return Err(CatalogError::Conflict.into());
        }
        Uuid::parse_str(&request).map_err(|_| CatalogError::Corrupt.into())
    })
    .transpose()
}
