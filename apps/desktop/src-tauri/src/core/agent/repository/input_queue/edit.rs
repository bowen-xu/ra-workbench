//! Original queue edit body. The facade owns BEGIN, rollback, proof and COMMIT.
use super::*;
use crate::agent::repository::sql_error::SqlRepositoryError;

pub(crate) type QueueSqlError = SqlRepositoryError<Error>;
type Result<T> = std::result::Result<T, QueueSqlError>;
fn domain(error: impl Into<Error>) -> QueueSqlError {
    QueueSqlError::Domain(error.into())
}

pub(crate) fn require_open_sql(db: &Connection, conversation: Uuid) -> Result<()> {
    if conversation.is_nil() {
        return Err(domain(QueueError::Invalid));
    }
    let open: bool = db.query_row(
        "select exists(select 1 from agent_conversations where id=?1 and archived=0)",
        [conversation.to_string()],
        |row| row.get(0),
    )?;
    if open {
        Ok(())
    } else {
        Err(domain(Error::Closed))
    }
}
pub(crate) fn load_sql(db: &Connection, conversation: Uuid) -> Result<InputQueue> {
    let saved: Option<(u64, String)> = db
        .query_row(
            "select revision,snapshot_json from agent_input_queues where conversation_id=?1",
            [conversation.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match saved {
        None => InputQueue::new(conversation).map_err(domain),
        Some((revision, json)) => {
            let queue = Snapshot::decode(&json, conversation).map_err(domain)?;
            if revision != queue.revision() {
                return Err(domain(Error::Corrupt));
            }
            Ok(queue)
        }
    }
}
pub(crate) fn read_edit_receipt_sql(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<(String, InputQueue)>> {
    let receipt: Option<(String,String,String)> = db.query_row(
        "select conversation_id,command_json,result_json from agent_input_queue_receipts where request_id=?1",
        [request.to_string()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    receipt
        .map(|(scope, command, result)| {
            if scope != conversation.to_string() {
                return Err(domain(QueueError::Conflict));
            }
            Ok((
                command,
                Snapshot::decode(&result, conversation).map_err(domain)?,
            ))
        })
        .transpose()
}

/// Rebuild only deterministic fields from the frozen result. No current queue,
/// source receipt or resource read can alter an already proven edit's result.
fn legacy_add_matches(prior: &str, saved: &InputQueue, revision: u64, edit: &QueueEdit) -> bool {
    match edit {
        QueueEdit::Add(input)
            if input.origin.is_some()
                && saved.inputs().iter().any(|old| {
                    old.id == input.id && old.text == input.text && old.origin.is_none()
                }) =>
        {
            let mut input = input.clone();
            input.origin = None;
            prior == command_json(revision, &QueueEdit::Add(input))
        }
        _ => false,
    }
}

pub(crate) fn read_frozen_edit_sql(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    edit: QueueEdit,
) -> Result<Option<InputQueue>> {
    read_equivalent_edit_sql(db, conversation, request, revision, edit, false)
}

/// An old Add may predate host provenance. This remains an explicitly legacy
/// observation; it must never manufacture a typed host fingerprint or proof.
pub(crate) fn read_legacy_edit_sql(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    edit: QueueEdit,
) -> Result<Option<InputQueue>> {
    read_equivalent_edit_sql(db, conversation, request, revision, edit, true)
}

fn read_equivalent_edit_sql(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    mut edit: QueueEdit,
    legacy: bool,
) -> Result<Option<InputQueue>> {
    let Some((command, saved)) = read_edit_receipt_sql(db, conversation, request)? else {
        return Ok(None);
    };
    if saved.revision()
        != revision
            .checked_add(1)
            .ok_or_else(|| domain(Error::Corrupt))?
    {
        return Err(domain(Error::Corrupt));
    }
    if let QueueEdit::Restore { input, paused, .. } = &mut edit {
        let restored = saved
            .inputs()
            .iter()
            .find(|old| old.id == input.id)
            .ok_or_else(|| domain(Error::Corrupt))?;
        if input.text != restored.text || input.user_input != restored.user_input {
            return Err(domain(QueueError::Conflict));
        }
        input.origin.clone_from(&restored.origin);
        *paused = saved.input_paused(input.id);
    }
    let old_add = legacy && legacy_add_matches(&command, &saved, revision, &edit);
    if command != command_json(revision, &edit) && !old_add {
        return Err(domain(QueueError::Conflict));
    }
    if !effect_matches(&saved, &edit, old_add) {
        return Err(domain(Error::Corrupt));
    }
    Ok(Some(saved))
}

fn effect_matches(queue: &InputQueue, edit: &QueueEdit, old_add: bool) -> bool {
    match edit {
        QueueEdit::Add(input) => queue.inputs().last().is_some_and(|saved| {
            saved.id == input.id
                && saved.text == input.text
                && saved.user_input == input.user_input
                && (saved.origin == input.origin || old_add && saved.origin.is_none())
        }),
        QueueEdit::Restore {
            input,
            source_id,
            index,
            paused,
        } => {
            queue.preparing().is_none()
                && !queue.inputs().iter().any(|saved| saved.id == *source_id)
                && queue
                    .inputs()
                    .get((*index).min(queue.inputs().len().saturating_sub(1)))
                    == Some(input)
                && queue.input_paused(input.id) == *paused
        }
        QueueEdit::Update {
            id,
            text,
            user_input,
        } => queue
            .inputs()
            .iter()
            .any(|saved| saved.id == *id && saved.text == *text && saved.user_input == *user_input),
        QueueEdit::Remove(id) => !queue.inputs().iter().any(|saved| saved.id == *id),
        QueueEdit::Reorder(ids) => {
            queue.preparing().is_none()
                && queue
                    .inputs()
                    .iter()
                    .map(|saved| saved.id)
                    .eq(ids.iter().copied())
        }
        QueueEdit::Pause => queue.paused(),
        QueueEdit::Resume => !queue.paused() && queue.preparing().is_none(),
        QueueEdit::PauseInput(id) => queue.input_paused(*id),
        QueueEdit::ResumeInput(id) => {
            queue.preparing().is_none()
                && !queue.paused()
                && !queue.input_paused(*id)
                && queue.inputs().iter().any(|saved| saved.id == *id)
        }
        QueueEdit::Prepare(_) | QueueEdit::Release(_) | QueueEdit::Admitted(_) => false,
    }
}

pub(crate) fn apply_in_tx_sql(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    edit: QueueEdit,
    authorize: impl FnOnce(&Connection) -> Result<()>,
) -> Result<InputQueue> {
    apply_in_tx(tx, conversation, request, revision, edit, authorize, false)
}

/// Only the verified handoff completion transaction may remove its own input.
pub(crate) fn apply_handoff_removal_in_tx_sql(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input: Uuid,
) -> Result<InputQueue> {
    apply_in_tx(
        tx,
        conversation,
        request,
        revision,
        QueueEdit::Remove(input),
        |_| Ok(()),
        true,
    )
}

fn apply_in_tx(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    mut edit: QueueEdit,
    authorize: impl FnOnce(&Connection) -> Result<()>,
    finishing_handoff: bool,
) -> Result<InputQueue> {
    if request.is_nil() {
        return Err(domain(QueueError::Invalid));
    }
    require_open_sql(tx, conversation)?;
    authorize(tx)?;
    super::restore::restore_origin_sql(tx, conversation, &mut edit)?;
    let command = command_json(revision, &edit);
    if let Some((prior, saved)) = read_edit_receipt_sql(tx, conversation, request)? {
        let legacy = legacy_add_matches(&prior, &saved, revision, &edit);
        if prior != command && !legacy {
            return Err(domain(QueueError::Conflict));
        }
        return Ok(saved);
    }
    if !finishing_handoff {
        super::handoff_guard::require_edit_unclaimed(tx, conversation, &edit)?;
    }
    if matches!(edit, QueueEdit::Admitted(_)) {
        return Err(domain(QueueError::Invalid));
    }
    if let QueueEdit::Add(input) | QueueEdit::Restore { input, .. } = &edit {
        let exists: bool = tx.query_row(
            "select exists(select 1 from agent_input_queue_identities where input_id=?1)",
            [input.id.to_string()],
            |row| row.get(0),
        )?;
        if exists {
            return Err(domain(QueueError::Conflict));
        }
        tx.execute(
            "insert into agent_input_queue_identities(input_id,conversation_id) values(?1,?2)",
            params![input.id.to_string(), conversation.to_string()],
        )?;
    }
    let queue = load_sql(tx, conversation)?
        .apply(revision, edit)
        .map_err(domain)?;
    let snapshot = Snapshot::encode(&queue).map_err(domain)?;
    tx.execute("insert into agent_input_queues(conversation_id,revision,snapshot_json) values(?1,?2,?3) on conflict(conversation_id) do update set revision=excluded.revision,snapshot_json=excluded.snapshot_json",
        params![conversation.to_string(),queue.revision(),snapshot])?;
    tx.execute("insert into agent_input_queue_receipts(request_id,conversation_id,command_json,result_json) values(?1,?2,?3,?4)",
        params![request.to_string(),conversation.to_string(),command,snapshot])?;
    Ok(queue)
}
