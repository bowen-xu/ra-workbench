//! Explicit preview cancellation retains input and records an idempotent receipt.
use super::*;
use crate::data_interface::SqlMutationError;
use research_agent_core::conversation::CatalogError;

pub fn cancel_preview_checked(
    db: &mut Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    preview_request: Uuid,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<InputQueue, Error> {
    if request.is_nil() || input_id.is_nil() || preview_request.is_nil() {
        return Err(QueueError::Invalid.into());
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    require_open(&tx, conversation)?;
    authorize(&tx)?;
    let queue = cancel_preview_in(
        &tx,
        conversation,
        request,
        revision,
        input_id,
        preview_request,
    )
    .map_err(legacy_error)?;
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(queue)
}

fn command(revision: u64, input: Uuid, preview: Uuid) -> String {
    serde_json::json!([1, revision, ["cancelPreview", input, preview]]).to_string()
}

/// This exact historical business receipt is not a typed SQL proof. A facade
/// must also verify the frozen manuscript scope and its own receipt family.
pub(crate) fn read_cancel_preview_receipt(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    preview_request: Uuid,
) -> Result<Option<InputQueue>, SqlMutationError> {
    let prior:Option<(String,String,String)>=db.query_row("select conversation_id,command_json,result_json from agent_input_queue_receipts where request_id=?1",[request.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((scope, saved, json)) = prior else {
        return Ok(None);
    };
    if scope != conversation.to_string() || saved != command(revision, input_id, preview_request) {
        return Err(CatalogError::Conflict.into());
    }
    let queue = Snapshot::decode(&json, conversation).map_err(domain)?;
    if queue.revision() != revision.checked_add(1).ok_or(CatalogError::Corrupt)?
        || !queue.paused()
        || queue.preparing().is_some()
        || !queue.inputs().iter().any(|input| input.id == input_id)
    {
        return Err(CatalogError::Corrupt.into());
    }
    Ok(Some(queue))
}

/// Caller owns commit and the ordinary preview's manuscript authorization.
pub(crate) fn cancel_preview_in(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    preview_request: Uuid,
) -> Result<InputQueue, SqlMutationError> {
    if [conversation, request, input_id, preview_request]
        .iter()
        .any(Uuid::is_nil)
    {
        return Err(CatalogError::InvalidInput.into());
    }
    require_open(tx, conversation).map_err(domain)?;
    // Compact retains its claim through task termination. The ordinary preview
    // cancel command cannot release that claim or bypass typed settlement.
    if crate::agent::writing_submission::read_compaction_revision_in(
        tx,
        conversation,
        preview_request,
    )
    .map_err(|error| match error {
        crate::agent::writing_submission::WritingSubmissionError::Input(
            research_agent_core::submission::SubmissionError::Corrupt,
        ) => CatalogError::Corrupt,
        _ => CatalogError::Unavailable,
    })?
    .is_some()
    {
        return Err(CatalogError::Conflict.into());
    }
    release_preview_in(
        tx,
        conversation,
        request,
        revision,
        input_id,
        preview_request,
    )
}

/// The typed compact facade must prove an unstarted, frozen compact claim
/// before using the same release and business receipt as ordinary cancellation.
pub(super) fn release_preview_in(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    preview_request: Uuid,
) -> Result<InputQueue, SqlMutationError> {
    if let Some(saved) = read_cancel_preview_receipt(
        tx,
        conversation,
        request,
        revision,
        input_id,
        preview_request,
    )? {
        return Ok(saved);
    }
    let linked: bool=tx.query_row(
        "select exists(select 1 from agent_input_queue_previews where input_id=?1 and conversation_id=?2 and request_id=?3)",
        params![input_id.to_string(),conversation.to_string(),preview_request.to_string()],|row|row.get(0))?;
    let admitted: bool = tx.query_row(
        "select exists(select 1 from agent_input_queue_admissions where input_id=?1)",
        [input_id.to_string()],
        |row| row.get(0),
    )?;
    if !linked || admitted {
        return Err(CatalogError::Conflict.into());
    }
    let queue = load(tx, conversation)
        .map_err(domain)?
        .apply(revision, QueueEdit::Release(input_id))
        .map_err(|error| domain(error.into()))?;
    let json = Snapshot::encode(&queue).map_err(domain)?;
    tx.execute(
        "update agent_input_queues set revision=?2,snapshot_json=?3 where conversation_id=?1",
        params![conversation.to_string(), queue.revision(), json],
    )?;
    tx.execute(
        "delete from agent_input_queue_previews where input_id=?1",
        [input_id.to_string()],
    )?;
    tx.execute("insert into agent_input_queue_receipts(request_id,conversation_id,command_json,result_json) values(?1,?2,?3,?4)",
        params![request.to_string(),conversation.to_string(),command(revision,input_id,preview_request),json])?;
    Ok(queue)
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
fn legacy_error(error: SqlMutationError) -> Error {
    match error {
        SqlMutationError::Sql(_) => Error::Storage,
        SqlMutationError::Domain(error) => match error {
            CatalogError::InvalidInput => QueueError::Invalid.into(),
            CatalogError::Conflict => QueueError::Conflict.into(),
            CatalogError::Busy => QueueError::Busy.into(),
            CatalogError::TooLarge => QueueError::Capacity.into(),
            CatalogError::NotFound => Error::Closed,
            CatalogError::Corrupt => Error::Corrupt,
            CatalogError::Unavailable => Error::Storage,
        },
    }
}

#[cfg(test)]
#[path = "cancel_preview/tests.rs"]
mod tests;
