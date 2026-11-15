//! Compact uses the existing cancellation family only after typed preparation.
use super::*;
use crate::agent::repository::input_queue::queued_compaction;
use rusqlite::{OptionalExtension, Transaction};

pub(super) fn frozen_scope(
    db: &Connection,
    command: &CancelPreviewIntent,
) -> Result<bool, SqlMutationError> {
    queued_compaction::frozen_scope_in(
        db,
        command.conversation_id,
        command.manuscript_id,
        command.input_id,
        command.preview_request_id,
    )
}

pub(super) fn check_prepare_proof(
    db: &Connection,
    command: &CancelPreviewIntent,
    fence: ReadFence,
) -> Result<bool, SqlImmediateFailure> {
    if writing_submission::read_compaction_revision_in(
        db,
        command.conversation_id,
        command.preview_request_id,
    )
    .map_err(|cause| error(command, fence, "queue-cancel-prepare", writing_error(cause)))?
    .is_none()
    {
        return Ok(false);
    }
    let raw: Option<String> = db.query_row(
        "select command_json from agent_input_queue_receipts where request_id=?1 and conversation_id=?2",
        rusqlite::params![command.preview_request_id.to_string(), command.conversation_id.to_string()],
        |row| row.get(0),
    ).optional().map_err(|cause| receipts::storage(command.request_id, fence, "queue-cancel-prepare", &cause))?;
    let raw = raw.ok_or(SqlImmediateFailure::Domain(CatalogError::Conflict))?;
    let identity =
        super::super::queued_compaction::prepare_receipt_identity(command.preview_request_id, &raw);
    receipts::check_cross_kind(
        db,
        command.preview_request_id,
        fence,
        "queue-cancel-prepare",
    )?;
    if receipts::lookup(db, &identity, fence, "queue-cancel-prepare")?.is_none() {
        return Err(SqlImmediateFailure::Domain(CatalogError::Conflict));
    }
    Ok(true)
}

pub(super) fn cancel_in(
    tx: &Transaction<'_>,
    command: &CancelPreviewIntent,
) -> Result<InputQueue, SqlMutationError> {
    if writing_submission::read_compaction_revision_in(
        tx,
        command.conversation_id,
        command.preview_request_id,
    )
    .map_err(writing_error)?
    .is_some()
    {
        queued_compaction::cancel_unstarted_in(
            tx,
            command.conversation_id,
            command.manuscript_id,
            command.request_id,
            command.expected_queue_revision,
            command.input_id,
            command.preview_request_id,
        )
    } else {
        input_queue::cancel_preview_in(
            tx,
            command.conversation_id,
            command.request_id,
            command.expected_queue_revision,
            command.input_id,
            command.preview_request_id,
        )
    }
}
