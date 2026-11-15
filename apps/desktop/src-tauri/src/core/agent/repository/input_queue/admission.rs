//! Admission and queue removal share one transaction. This creates an immutable
//! submission; it never starts a model call or retries an execution.
use super::*;
use crate::agent::repository::submissions;
use research_agent_core::submission::{SubmissionError, SubmissionInput, SubmissionRecord};

pub fn admit_checked(
    db: &mut Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    input: SubmissionInput,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<(SubmissionRecord, InputQueue), Error> {
    admit_checked_with_images(
        db,
        conversation,
        request,
        revision,
        input_id,
        input,
        &[],
        authorize,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "atomic queue admission keeps all transaction inputs explicit"
)]
pub fn admit_checked_with_images(
    db: &mut Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    input: SubmissionInput,
    images: &[research_agent_core::submission::SubmissionImage],
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<(SubmissionRecord, InputQueue), Error> {
    admit_with_policy(
        db,
        conversation,
        request,
        revision,
        input_id,
        input,
        images,
        false,
        authorize,
    )
}

/// Explicitly continuing a frozen queue head may resume its queue in the same
/// admission transaction. Exact receipt replay never resumes a later pause.
pub(crate) fn admit_and_resume_checked(
    db: &mut Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    input: SubmissionInput,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<(SubmissionRecord, InputQueue), Error> {
    admit_with_policy(
        db,
        conversation,
        request,
        revision,
        input_id,
        input,
        &[],
        true,
        authorize,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "one transaction owns queue admission and an explicit resume"
)]
fn admit_with_policy(
    db: &mut Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    input: SubmissionInput,
    images: &[research_agent_core::submission::SubmissionImage],
    resume: bool,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<(SubmissionRecord, InputQueue), Error> {
    if request.is_nil() || input_id.is_nil() {
        return Err(QueueError::Invalid.into());
    }
    input.validate().map_err(submission_error)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    let result = admit_in_tx(
        &tx,
        conversation,
        request,
        revision,
        input_id,
        input,
        images,
        resume,
        authorize,
    )?;
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(result)
}

/// Caller owns the complete transaction, including commit and rollback.
#[expect(
    clippy::too_many_arguments,
    reason = "the caller freezes the full queue admission intent"
)]
pub(crate) fn admit_in_tx(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    input: SubmissionInput,
    images: &[research_agent_core::submission::SubmissionImage],
    resume: bool,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<(SubmissionRecord, InputQueue), Error> {
    if request.is_nil() || input_id.is_nil() {
        return Err(QueueError::Invalid.into());
    }
    input.validate().map_err(submission_error)?;
    require_open(tx, conversation)?;
    authorize(tx)?;
    let prior: Option<(String,String,u64,String)> = tx.query_row(
        "select conversation_id,request_id,expected_revision,result_json from agent_input_queue_admissions where input_id=?1",
        [input_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
        .optional().map_err(|_| Error::Storage)?;
    if let Some((scope, prior_request, expected, result)) = prior {
        if scope != conversation.to_string()
            || prior_request != request.to_string()
            || expected != revision
        {
            return Err(QueueError::Conflict.into());
        }
        let record = submissions::read(tx, conversation, request)
            .map_err(submission_error)?
            .ok_or(Error::Corrupt)?;
        if record.input != input {
            return Err(QueueError::Conflict.into());
        }
        return Ok((record, Snapshot::decode(&result, conversation)?));
    }
    handoff_guard::require_unclaimed(tx, conversation, input_id)
        .map_err(|cause| cause.domain_or(Error::Storage))?;
    // A normal or another queued submission may already own this request. It
    // cannot become a second input's admission even when its text is identical.
    let prepared = if let Some(record) =
        submissions::read(tx, conversation, request).map_err(submission_error)?
    {
        let linked: bool = tx.query_row(
            "select exists(select 1 from agent_input_queue_previews where input_id=?1 and conversation_id=?2 and request_id=?3)",
            params![input_id.to_string(),conversation.to_string(),request.to_string()], |row|row.get(0)).map_err(|_|Error::Storage)?;
        if !linked || record.input != input {
            return Err(QueueError::Conflict.into());
        }
        true
    } else {
        false
    };
    let queue = load(tx, conversation)?;
    if queue.preparing() != Some(input_id)
        || queue
            .next_input()
            // A linked preview validated the original composer payload while
            // claiming this immutable head; its source identities may be resolved.
            .is_none_or(|head| {
                head.text != input.text || !prepared && head.user_input != input.user_input
            })
    {
        return Err(QueueError::Conflict.into());
    }
    let mut next = queue.apply(revision, QueueEdit::Admitted(input_id))?;
    if resume && next.paused() {
        next = next.apply(next.revision(), QueueEdit::Resume)?;
    }
    let record =
        submissions::insert_in(tx, conversation, request, input).map_err(submission_error)?;
    submissions::insert_image_manifest(tx, conversation, request, images)
        .map_err(submission_error)?;
    let snapshot = Snapshot::encode(&next)?;
    tx.execute("insert or ignore into agent_queue_submission_origins(conversation_id,request_id,input_id) values(?1,?2,?3)",
        params![conversation.to_string(),request.to_string(),input_id.to_string()]).map_err(|_|Error::Storage)?;
    tx.execute(
        "update agent_input_queues set revision=?2,snapshot_json=?3 where conversation_id=?1",
        params![conversation.to_string(), next.revision(), snapshot],
    )
    .map_err(|_| Error::Storage)?;
    tx.execute("insert into agent_input_queue_admissions(input_id,conversation_id,request_id,expected_revision,result_json) values(?1,?2,?3,?4,?5)",
        params![input_id.to_string(),conversation.to_string(),request.to_string(),revision,snapshot]).map_err(|_| Error::Storage)?;
    Ok((record, next))
}

pub(super) fn submission_error(error: SubmissionError) -> Error {
    match error {
        SubmissionError::Conflict => QueueError::Conflict.into(),
        SubmissionError::InvalidInput => QueueError::Invalid.into(),
        SubmissionError::Corrupt => Error::Corrupt,
        _ => Error::Storage,
    }
}

#[cfg(test)]
#[path = "admission/tests.rs"]
mod tests;
