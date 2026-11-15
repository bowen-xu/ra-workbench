//! Ordinary preview preparation facts. The data owner supplies the transaction.
use super::*;
use crate::{
    agent::writing_context::{ContextNeed, WritingContextReader},
    ai::structured_resource_validation::Flow,
};
use crate::{
    agent::{repository::submissions, writing_submission},
    data_interface::SqlMutationError,
};
use research_agent_core::{conversation::CatalogError, submission::SubmissionError};
use rusqlite::Transaction;
#[path = "preview_prepare/types.rs"]
mod types;
pub(crate) use types::{
    FrozenPrepareCommand, QueuePreviewPrepareIntent, QueuePreviewPrepareSource,
    QueuePreviewPrepared, command, command_fingerprint,
};

pub(crate) enum Preparation {
    Missing,
    Legacy,
    Prepared(Box<QueuePreviewPrepared>),
}
#[derive(Debug)]
pub(crate) enum PrepareError {
    Mutation(SqlMutationError),
    Materialization(Box<writing_submission::WritingSubmissionError>),
    Grants,
}
impl From<PrepareError> for Flow<PrepareError, ContextNeed> {
    fn from(error: PrepareError) -> Self {
        Self::original(error)
    }
}
impl From<SqlMutationError> for Flow<PrepareError, ContextNeed> {
    fn from(error: SqlMutationError) -> Self {
        Self::original(error.into())
    }
}
impl From<SqlMutationError> for PrepareError {
    fn from(cause: SqlMutationError) -> Self {
        Self::Mutation(cause)
    }
}

pub(crate) fn authorize(
    db: &Connection,
    intent: &QueuePreviewPrepareIntent,
) -> Result<(), SqlMutationError> {
    writing_submission::check_binding(db, intent.conversation_id, intent.manuscript_id)
        .map_err(writing)
}

fn ordinary(db: &Connection, intent: &QueuePreviewPrepareIntent) -> Result<(), SqlMutationError> {
    if writing_submission::read_compaction_revision_in(
        db,
        intent.conversation_id,
        intent.request_id,
    )
    .map_err(writing)?
    .is_some()
    {
        return Err(CatalogError::Conflict.into());
    }
    Ok(())
}

/// Immutable evidence is independent of the current pointer and queue revision.
/// This does not authorize a write or turn start against a revoked binding.
pub(crate) fn read_in(
    db: &Connection,
    intent: &QueuePreviewPrepareIntent,
) -> Result<Preparation, SqlMutationError> {
    let expected = command(intent)?;
    ordinary(db, intent)?;
    let prior: Option<(String, String, String)> = db.query_row(
        "select conversation_id,command_json,result_json from agent_input_queue_receipts where request_id=?1",
        [intent.request_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    let Some((scope, saved, json)) = prior else {
        let legacy: bool = db.query_row(
            "select exists(select 1 from agent_submissions where request_id=?1)
             or exists(select 1 from agent_input_queue_previews where request_id=?1)
             or exists(select 1 from agent_queue_submission_origins where request_id=?1)",
            [intent.request_id.to_string()],
            |row| row.get(0),
        )?;
        return Ok(if legacy {
            Preparation::Legacy
        } else {
            Preparation::Missing
        });
    };
    if scope != intent.conversation_id.to_string() || saved != expected {
        return Err(CatalogError::Conflict.into());
    }
    let queue = Snapshot::decode(&json, intent.conversation_id).map_err(domain)?;
    let record = submissions::read(db, intent.conversation_id, intent.request_id)
        .map_err(submission)?
        .ok_or(CatalogError::Corrupt)?;
    // The proof binds a frozen manuscript scope, even when live authorization
    // has since changed. Inspection must not adopt a differently scoped record.
    writing_submission::validate_scope(&record, intent.manuscript_id)
        .map_err(|_| CatalogError::Corrupt)?;
    let linked: bool = db.query_row(
        "select exists(select 1 from agent_queue_submission_origins
         where conversation_id=?1 and request_id=?2 and input_id=?3)",
        params![
            intent.conversation_id.to_string(),
            intent.request_id.to_string(),
            intent.input_id.to_string()
        ],
        |row| row.get(0),
    )?;
    if !linked
        || queue.revision() != intent.expected_queue_revision + 1
        || queue.preparing() != Some(intent.input_id)
        || queue.paused()
        || queue
            .next_input()
            .is_none_or(|input| input.id != intent.input_id || input.text != record.input.text)
    {
        return Err(CatalogError::Corrupt.into());
    }
    Ok(Preparation::Prepared(Box::new(QueuePreviewPrepared {
        record,
        queue,
    })))
}

/// Check fresh eligibility before the service resolves mutable model settings.
/// The transaction repeats this check after materialization and before commit.
pub(crate) fn require_fresh(
    db: &Connection,
    intent: &QueuePreviewPrepareIntent,
) -> Result<(), SqlMutationError> {
    handoff_guard::require_unclaimed(db, intent.conversation_id, intent.input_id).map_err(
        |cause| match cause {
            crate::agent::repository::SqlRepositoryError::Sql(error) => {
                SqlMutationError::from(error)
            }
            crate::agent::repository::SqlRepositoryError::Domain(error) => domain(error),
        },
    )?;
    let queue = load(db, intent.conversation_id).map_err(domain)?;
    if queue
        .next_input()
        .is_none_or(|input| input.id != intent.input_id)
    {
        return Err(CatalogError::Conflict.into());
    }
    queue
        .apply(
            intent.expected_queue_revision,
            QueueEdit::Prepare(intent.input_id),
        )
        .map_err(|cause| domain(cause.into()))?;
    let linked: bool = db.query_row(
        "select exists(select 1 from agent_input_queue_previews where input_id=?1)",
        [intent.input_id.to_string()],
        |row| row.get(0),
    )?;
    if linked {
        return Err(CatalogError::Conflict.into());
    }
    Ok(())
}

pub(crate) fn prepare_in(
    tx: &Transaction<'_>,
    intent: &QueuePreviewPrepareIntent,
    source: QueuePreviewPrepareSource,
) -> Result<QueuePreviewPrepared, PrepareError> {
    prepare_with_context_in(tx, intent, source, None).map_err(|flow| match flow {
        Flow::Fail(failure) => failure.error,
        Flow::Need(_) => unreachable!("static queue preparation cannot pause for Native"),
    })
}

pub(crate) fn prepare_with_context_in(
    tx: &Transaction<'_>,
    intent: &QueuePreviewPrepareIntent,
    source: QueuePreviewPrepareSource,
    writing: Option<&mut dyn WritingContextReader>,
) -> Result<QueuePreviewPrepared, Flow<PrepareError, ContextNeed>> {
    let command = command(intent)?;
    if source.request.conversation_id != intent.conversation_id
        || source.request.request_id != intent.request_id
        || source.request.context.manuscript_id != intent.manuscript_id
    {
        return Err(SqlMutationError::from(CatalogError::InvalidInput).into());
    }
    require_fresh(tx, intent)?;
    let original = source.request.user_input.clone();
    let input = writing_submission::materialize_submission_with_context(
        tx,
        &source.store,
        source.request,
        source.model,
        writing,
    )
    .map_err(|flow| flow.map_error(|cause| PrepareError::Materialization(Box::new(cause))))?;
    // Empty source selection is invalid user input. Reject before installing a
    // claim that the existing restore/start/cancel scope checks cannot recover.
    if input.context.is_empty() {
        return Err(PrepareError::Materialization(Box::new(
            crate::agent::writing_context::WritingContextError::InvalidInput.into(),
        ))
        .into());
    }
    let (record, queue) = prepare_resolved_preview_in(
        tx,
        intent.conversation_id,
        intent.request_id,
        intent.expected_queue_revision,
        intent.input_id,
        input,
        &original,
    )
    .map_err(domain)?;
    crate::agent::skills::steering::freeze_grants(tx, &source.store, &record)
        .map_err(|_| PrepareError::Grants)?;
    let json = Snapshot::encode(&queue).map_err(domain)?;
    tx.execute(
        "insert into agent_input_queue_receipts(request_id,conversation_id,command_json,result_json) values(?1,?2,?3,?4)",
        params![intent.request_id.to_string(), intent.conversation_id.to_string(), command, json],
    ).map_err(SqlMutationError::from)?;
    Ok(QueuePreviewPrepared { record, queue })
}

fn domain(cause: Error) -> SqlMutationError {
    match cause {
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
fn submission(cause: SubmissionError) -> SqlMutationError {
    match cause {
        SubmissionError::Conflict => CatalogError::Conflict,
        SubmissionError::InvalidInput => CatalogError::InvalidInput,
        SubmissionError::Corrupt => CatalogError::Corrupt,
        _ => CatalogError::Unavailable,
    }
    .into()
}
fn writing(cause: writing_submission::WritingSubmissionError) -> SqlMutationError {
    match cause {
        writing_submission::WritingSubmissionError::Unbound => CatalogError::NotFound.into(),
        writing_submission::WritingSubmissionError::Input(cause) => submission(cause),
        _ => CatalogError::Unavailable.into(),
    }
}
