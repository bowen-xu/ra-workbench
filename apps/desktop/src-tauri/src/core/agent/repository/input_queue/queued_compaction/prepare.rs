use super::*;
use crate::{
    agent::writing_context::{ContextNeed, WritingContextReader},
    ai::structured_resource_validation::Flow,
};

impl From<SqlMutationError> for Flow<SqlMutationError, ContextNeed> {
    fn from(error: SqlMutationError) -> Self {
        Self::original(error)
    }
}
impl From<rusqlite::Error> for Flow<SqlMutationError, ContextNeed> {
    fn from(error: rusqlite::Error) -> Self {
        Self::original(error.into())
    }
}
impl From<CatalogError> for Flow<SqlMutationError, ContextNeed> {
    fn from(error: CatalogError) -> Self {
        Self::original(error.into())
    }
}

pub(crate) fn replay_in(
    db: &Connection,
    intent: &PrepareIntent,
) -> Result<Option<QueuedCompactionState>, SqlMutationError> {
    authorize(db, intent.conversation_id, intent.manuscript_id)?;
    let command = prepare_command(intent)?;
    let prior: Option<String> = db
        .query_row(
            "select command_json from agent_input_queue_receipts where request_id=?1",
            [intent.request_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(prior) = prior else { return Ok(None) };
    if command != prior {
        return Err(CatalogError::Conflict.into());
    }
    // A prepare receipt acknowledges its original Q+1 and purpose, without
    // consulting a newer queue pointer or task settlement.
    let (claim, frozen) = read::preparation(
        db,
        intent.conversation_id,
        intent.input_id,
        intent.request_id,
    )?
    .ok_or(CatalogError::Corrupt)?;
    let record = saved(db, intent.conversation_id, intent.request_id)?;
    let expected = crate::agent::writing_submission::read_compaction_revision_in(
        db,
        intent.conversation_id,
        intent.request_id,
    )
    .map_err(writing)?;
    if expected != Some(claim.expected_revision) || record.input.text.trim() != "/compact" {
        return Err(CatalogError::Corrupt.into());
    }
    Ok(Some(QueuedCompactionState {
        record,
        queue: frozen,
        claim_revision: claim.expected_queue_revision,
        expected_revision: claim.expected_revision,
        outcome: None,
        current: true,
        retirement: None,
    }))
}

pub(crate) fn prepare_in(
    tx: &Transaction<'_>,
    intent: &PrepareIntent,
    source: PrepareSource,
    previous_retirement: Option<&crate::data_interface::QueuePreviewRetirement>,
) -> Result<QueuedCompactionState, SqlMutationError> {
    prepare_with_context_in(tx, intent, source, previous_retirement, None).map_err(
        |flow| match flow {
            Flow::Fail(failure) => failure.error,
            Flow::Need(_) => unreachable!("static queue compaction cannot pause for Native"),
        },
    )
}

pub(crate) fn prepare_with_context_in(
    tx: &Transaction<'_>,
    intent: &PrepareIntent,
    source: PrepareSource,
    previous_retirement: Option<&crate::data_interface::QueuePreviewRetirement>,
    writing_source: Option<&mut dyn WritingContextReader>,
) -> Result<QueuedCompactionState, Flow<SqlMutationError, ContextNeed>> {
    if let Some(state) = replay_in(tx, intent)? {
        return Ok(state);
    }
    let command = prepare_command(intent)?;
    let conversation = intent.conversation_id;
    if source.request.conversation_id != conversation
        || source.request.request_id != intent.request_id
        || source.request.context.manuscript_id != intent.manuscript_id
        || source.request.text.trim() != "/compact"
    {
        return Err(CatalogError::InvalidInput.into());
    }
    super::super::handoff_guard::require_unclaimed(tx, conversation, intent.input_id).map_err(
        |cause| match cause {
            crate::agent::repository::SqlRepositoryError::Sql(error) => {
                SqlMutationError::from(error)
            }
            crate::agent::repository::SqlRepositoryError::Domain(error) => domain(error),
        },
    )?;
    let q = queue(tx, conversation)?;
    if q.revision() != intent.expected_queue_revision
        || q.next_input().is_none_or(|head| {
            head.id != intent.input_id
                || head.text != source.request.text
                || head.user_input != source.request.user_input
        })
    {
        return Err(CatalogError::Conflict.into());
    }
    if current_request(tx, conversation, intent.input_id)?.is_some() {
        return Err(CatalogError::Busy.into());
    }
    let previous:Option<String>=tx.query_row("select request_id from agent_queue_submission_origins where conversation_id=?1 and input_id=?2 order by rowid desc limit 1",params![conversation.to_string(),intent.input_id.to_string()],|row|row.get(0)).optional()?;
    match (intent.previous_request_id, previous) {
        (None, None) => read::reject_legacy_candidate(tx, conversation, intent.input_id)?,
        (Some(expected), Some(saved_request)) if saved_request == expected.to_string() => {
            let old = saved(tx, conversation, expected)?;
            let (_, old_snapshot) = read::preparation(tx, conversation, intent.input_id, expected)?
                .ok_or(CatalogError::Conflict)?;
            let retired = previous_retirement.is_some_and(|proof| {
                proof.intent.conversation_id == conversation
                    && proof.intent.manuscript_id == intent.manuscript_id
                    && proof.intent.input_id == intent.input_id
                    && proof.intent.preview_request_id == expected
                    && proof.receipt.operation_id == proof.intent.request_id
            });
            if old_snapshot.preparing() != Some(intent.input_id)
                || !(retired
                    || matches!(
                        read::terminal(tx, &old)?,
                        Some(
                            TurnOutcome::Failed | TurnOutcome::Canceled | TurnOutcome::Interrupted
                        )
                    ))
            {
                return Err(CatalogError::Conflict.into());
            }
        }
        _ => return Err(CatalogError::Conflict.into()),
    }
    if submissions::read(tx, conversation, intent.request_id)
        .map_err(|error| domain(admission::submission_error(error)))?
        .is_some()
    {
        return Err(CatalogError::Conflict.into());
    }
    let actual: u64 = tx.query_row(
        "select durable_count from agent_conversations where id=?1",
        [conversation.to_string()],
        |row| row.get(0),
    )?;
    if actual != intent.expected_revision {
        return Err(CatalogError::Conflict.into());
    }
    let next = q
        .apply(
            intent.expected_queue_revision,
            QueueEdit::Prepare(intent.input_id),
        )
        .map_err(|error| domain(error.into()))?;
    crate::agent::writing_submission::check_compaction_prepare_in(
        tx,
        conversation,
        intent.request_id,
        &source.request.text,
        Some(intent.expected_revision),
    )
    .map_err(writing)?;
    let input = crate::agent::writing_submission::materialize_submission_with_context(
        tx,
        &source.store,
        source.request,
        source.model,
        writing_source,
    )
    .map_err(|flow| flow.map_error(writing))?;
    let record = submissions::insert_in(tx, conversation, intent.request_id, input)
        .map_err(|error| domain(admission::submission_error(error)))?;
    crate::agent::writing_submission::freeze_compaction_in(
        tx,
        &record,
        Some(intent.expected_revision),
    )
    .map_err(writing)?;
    crate::agent::skills::steering::freeze_grants(tx, &source.store, &record)
        .map_err(|_| CatalogError::Unavailable)?;
    tx.execute("insert into agent_queue_submission_origins(conversation_id,request_id,input_id) values(?1,?2,?3)",params![conversation.to_string(),intent.request_id.to_string(),intent.input_id.to_string()])?;
    let json = save_queue(tx, &next)?;
    tx.execute("insert into agent_input_queue_previews(input_id,conversation_id,request_id,expected_revision,result_json) values(?1,?2,?3,?4,?5)",params![intent.input_id.to_string(),conversation.to_string(),intent.request_id.to_string(),intent.expected_queue_revision,json])?;
    tx.execute("insert into agent_input_queue_receipts(request_id,conversation_id,command_json,result_json) values(?1,?2,?3,?4)",params![intent.request_id.to_string(),conversation.to_string(),command,json])?;
    Ok(QueuedCompactionState {
        record,
        queue: next,
        claim_revision: intent.expected_queue_revision,
        expected_revision: intent.expected_revision,
        outcome: None,
        current: true,
        retirement: None,
    })
}
