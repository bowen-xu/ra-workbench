//! Authorization and durable queue admission, independent of transport and command handlers.
use super::{
    note_queue::Scope,
    note_submission::{self, source::SourceReceipt},
    queue_contract::{CommandError, Confirmation},
    repository::{input_queue, submissions},
};
use crate::storage::AppStore;
use research_agent_core::submission::SubmissionRecord;
use rusqlite::{OptionalExtension, params};

#[derive(Debug)]
pub(super) enum Error {
    ConfirmationMismatch,
    Queue(CommandError),
    RecoveryRequired,
}

pub(super) async fn check_origin(
    store: &AppStore,
    receipt: SourceReceipt,
    expected: &SubmissionRecord,
    queue: Option<Confirmation>,
) -> Result<(), Error> {
    if queue.is_some_and(|q| q.input_id.is_nil() || q.expected_revision >= 9_007_199_254_740_991) {
        return Err(Error::Queue(CommandError::Invalid));
    }
    let store = store.clone();
    let expected = expected.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_|Error::RecoveryRequired)?;
        let tx = db.transaction().map_err(|_|Error::RecoveryRequired)?;
        if authorize_record(&tx,receipt).map_err(|e|Error::Queue(e.into()))? != expected {
            return Err(Error::ConfirmationMismatch);
        }
        let origin:Option<String> = tx.query_row("select input_id from agent_queue_submission_origins where conversation_id=?1 and request_id=?2",params![receipt.conversation_id.to_string(),receipt.request_id.to_string()],|row|row.get(0)).optional().map_err(|_|Error::RecoveryRequired)?;
        match (origin,queue) {
            (None,None) => Ok(()),
            (Some(id),Some(queue)) if id==queue.input_id.to_string() => Ok(()),
            _ => Err(Error::ConfirmationMismatch),
        }
    }).await.map_err(|_|Error::RecoveryRequired)?
}

pub(super) async fn admit(
    store: &AppStore,
    receipt: SourceReceipt,
    expected: &SubmissionRecord,
    queue: Confirmation,
) -> Result<(), Error> {
    let store = store.clone();
    let expected = expected.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| Error::RecoveryRequired)?;
        let (record, _) = input_queue::admit_checked(
            &mut db,
            receipt.conversation_id,
            receipt.request_id,
            queue.expected_revision,
            queue.input_id,
            expected.input.clone(),
            |db| {
                if authorize_record(db, receipt)? != expected {
                    return Err(input_queue::Error::Corrupt);
                }
                Ok(())
            },
        )
        .map_err(|e| Error::Queue(e.into()))?;
        if record != expected {
            return Err(Error::ConfirmationMismatch);
        }
        Ok(())
    })
    .await
    .map_err(|_| Error::RecoveryRequired)?
}
/// Cancellation and admission also validate the frozen source inside their write
/// transaction; a second valid binding cannot relabel another resource's preview.
pub(super) fn authorize_record(
    db: &rusqlite::Connection,
    receipt: SourceReceipt,
) -> Result<SubmissionRecord, input_queue::Error> {
    Scope::from(receipt.resource).authorize(db, receipt.conversation_id)?;
    let record = submissions::read(db, receipt.conversation_id, receipt.request_id)
        .map_err(|_| input_queue::Error::Corrupt)?
        .ok_or(input_queue::Error::Closed)?;
    note_submission::validate_record(&record, receipt.resource)
        .map_err(|_| input_queue::Error::Closed)?;
    Ok(record)
}
