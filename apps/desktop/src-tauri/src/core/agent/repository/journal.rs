use research_agent_core::conversation::{ConversationId, HistoryPage, HistoryQuery};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Stable storage errors; do not expose database text or manuscript content.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    #[error("conversation not found")]
    NotFound,
    #[error("conversation already exists")]
    AlreadyExists,
    #[error("journal revision conflict")]
    RevisionConflict,
    #[error("archived conversation rejects new event batches")]
    Archived,
    #[error("agent_legacy_migration_incomplete: pending legacy import blocks new turn admission")]
    LegacyMigrationIncomplete,
    #[error("batch identity reused with different content")]
    BatchConflict,
    #[error("invalid journal input")]
    InvalidInput,
    #[error("journal storage unavailable")]
    Unavailable,
    #[error("journal content is corrupt or unsupported")]
    Corrupt,
}

impl From<rusqlite::Error> for JournalError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Unavailable
    }
}

#[cfg(test)]
pub fn create_conversation(
    connection: &mut Connection,
    id: ConversationId,
    title: &str,
) -> Result<(), JournalError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 180 {
        return Err(JournalError::InvalidInput);
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exists: bool = transaction.query_row(
        "select exists(select 1 from agent_conversations where id = ?1)",
        [id.as_uuid().to_string()],
        |row| row.get(0),
    )?;
    if exists {
        return Err(JournalError::AlreadyExists);
    }
    transaction.execute(
        "insert into agent_conversations (id, title) values (?1, ?2)",
        params![id.as_uuid().to_string(), title],
    )?;
    transaction.commit()?;
    Ok(())
}

/// Atomically append a bounded batch. Replaying the exact batch returns its original
/// watermark, even if subsequent batches have committed. Different content with the
/// same identity is rejected. A failed transaction changes neither history nor count.
#[cfg(test)]
pub fn append_batch<Item: Serialize>(
    connection: &mut Connection,
    id: ConversationId,
    batch_id: Uuid,
    expected_count: u64,
    items: &[Item],
) -> Result<u64, JournalError> {
    write_batch(connection, id, batch_id, expected_count, items, None)
}

/// Atomically materialize a lazy conversation together with its first batch.
pub(super) fn write_batch<Item: Serialize>(
    connection: &mut Connection,
    id: ConversationId,
    batch_id: Uuid,
    expected_count: u64,
    items: &[Item],
    new_title: Option<&str>,
) -> Result<u64, JournalError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = write_batch_in_tx(
        &transaction,
        JournalWrite {
            id,
            batch_id,
            expected_count,
            items,
            new_title,
            frozen_at: None,
        },
        |_, _, _| Ok(()),
    )?;
    transaction.commit()?;
    Ok(result)
}

pub(crate) struct JournalWrite<'a, Item> {
    pub id: ConversationId,
    pub batch_id: Uuid,
    pub expected_count: u64,
    pub items: &'a [Item],
    pub new_title: Option<&'a str>,
    /// Typed log replay supplies frozen UTC; the legacy wrapper keeps its
    /// deployed SQL timestamp behavior.
    pub frozen_at: Option<&'a str>,
}

/// Same journal invariants inside a caller-owned transaction. The callback runs
/// immediately before each physical event, allowing a projector to flush prior
/// deltas before completion/acceptance triggers observe their message body.
pub(crate) fn write_batch_in_tx<Item: Serialize>(
    transaction: &Transaction<'_>,
    write: JournalWrite<'_, Item>,
    mut before_insert: impl FnMut(&Transaction<'_>, u64, &Item) -> Result<(), JournalError>,
) -> Result<u64, JournalError> {
    let JournalWrite {
        id,
        batch_id,
        expected_count,
        items,
        new_title,
        frozen_at,
    } = write;
    if (items.is_empty() && new_title.is_none()) || items.len() > 500 {
        return Err(JournalError::InvalidInput);
    }
    if new_title.is_some_and(|title| title.trim().is_empty() || title.chars().count() > 180)
        || (new_title.is_some() && expected_count != 0)
    {
        return Err(JournalError::InvalidInput);
    }
    let expected = i64::try_from(expected_count).map_err(|_| JournalError::InvalidInput)?;
    let result = expected
        .checked_add(items.len() as i64)
        .ok_or(JournalError::InvalidInput)?;
    let payloads = items
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| JournalError::InvalidInput)?;
    let hash = hash_payloads(&payloads);
    let id = id.as_uuid().to_string();
    let batch_id = batch_id.to_string();
    if let Some(title) = new_title {
        let exists: bool = transaction.query_row(
            "select exists(select 1 from agent_conversations where id = ?1)",
            [&id],
            |row| row.get(0),
        )?;
        if exists {
            return Err(JournalError::AlreadyExists);
        }
        if let Some(at) = frozen_at {
            transaction.execute("insert into agent_conversations (id,title,created_at,updated_at) values (?1,?2,?3,?3)", params![id,title.trim(),at])?;
        } else {
            transaction.execute(
                "insert into agent_conversations (id, title) values (?1, ?2)",
                params![id, title.trim()],
            )?;
        }
    }
    let previous: Option<(i64, i64, String)> = transaction
        .query_row(
            "select expected_count, result_count, payload_hash from agent_journal_batches
         where conversation_id = ?1 and batch_id = ?2",
            params![id, batch_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((previous_expected, previous_result, previous_hash)) = previous {
        return if previous_expected == expected
            && previous_result == result
            && previous_hash == hash
        {
            Ok(result as u64)
        } else {
            Err(JournalError::BatchConflict)
        };
    }
    let (current, archived): (i64, bool) = transaction
        .query_row(
            "select durable_count, archived from agent_conversations where id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(JournalError::NotFound)?;
    // Exact committed replays above remain observable. An archive that wins
    // before a pending TurnStarted batch prevents that turn from being admitted.
    if archived {
        return Err(JournalError::Archived);
    }
    if current != expected {
        return Err(JournalError::RevisionConflict);
    }
    for payload in &payloads {
        let value: serde_json::Value =
            serde_json::from_str(payload).map_err(|_| JournalError::InvalidInput)?;
        if value["payload"]["type"] == "turnStarted" {
            let event: research_agent_core::protocol::AgentEvent =
                serde_json::from_value(value.clone()).map_err(|_| JournalError::InvalidInput)?;
            if event.conversation_id().as_uuid().to_string() != id {
                return Err(JournalError::InvalidInput);
            }
            validate_authority(transaction, &event)?;
        }
        if value["payload"]["type"] == "conversationRolledBack" {
            if payloads.len() != 1 || new_title.is_some() {
                return Err(JournalError::InvalidInput);
            }
            let event: research_agent_core::protocol::AgentEvent =
                serde_json::from_value(value).map_err(|_| JournalError::InvalidInput)?;
            if event.conversation_id().as_uuid().to_string() != id
                || event.seq() != expected_count + 1
            {
                return Err(JournalError::InvalidInput);
            }
            validate_authority(transaction, &event)?;
            super::context_windows::rollback_in(transaction, &event)?;
            super::input_queue::pause_for_history_rewrite_in(
                transaction,
                *event.conversation_id().as_uuid(),
            )
            .map_err(|error| match error {
                super::input_queue::Error::Storage => JournalError::Unavailable,
                super::input_queue::Error::Corrupt => JournalError::Corrupt,
                _ => JournalError::InvalidInput,
            })?;
        }
    }
    for (index, payload) in payloads.iter().enumerate() {
        before_insert(transaction, (expected + index as i64) as u64, &items[index])?;
        transaction.execute(
            "insert into agent_items (conversation_id, ordinal, schema_version, payload_json)
             values (?1, ?2, 1, ?3)",
            params![id, expected + index as i64, payload],
        )?;
    }
    if !items.is_empty() {
        transaction.execute(
            "insert into agent_journal_batches
         (conversation_id, batch_id, expected_count, result_count, payload_hash)
         values (?1, ?2, ?3, ?4, ?5)",
            params![id, batch_id, expected, result, hash],
        )?;
    }
    if let Some(at) = frozen_at {
        transaction.execute(
            "update agent_conversations set durable_count=?2,updated_at=?3 where id=?1",
            params![id, result, at],
        )?;
    } else {
        transaction.execute(
            "update agent_conversations set durable_count = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
            params![id, result],
        )?;
    }
    Ok(result as u64)
}

pub(crate) fn payload_hash<Item: Serialize>(items: &[Item]) -> Result<String, JournalError> {
    let payloads = items
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| JournalError::InvalidInput)?;
    Ok(hash_payloads(&payloads))
}

fn hash_payloads(payloads: &[String]) -> String {
    let mut digest = Sha256::new();
    for payload in payloads {
        digest.update((payload.len() as u64).to_be_bytes());
        digest.update(payload.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Read-only checks shared by pre-log admission and the committing transaction.
pub(crate) fn validate_authority(
    db: &Connection,
    event: &research_agent_core::protocol::AgentEvent,
) -> Result<(), JournalError> {
    use research_agent_core::protocol::EventPayload;
    match event.payload() {
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Compact,
            ..
        } => super::input_queue::queued_compaction::require_startable_in(
            db,
            *event.conversation_id().as_uuid(),
            event.request_id(),
        )
        .map_err(|error| match error {
            crate::data_interface::SqlMutationError::Domain(
                research_agent_core::conversation::CatalogError::Corrupt,
            ) => JournalError::Corrupt,
            crate::data_interface::SqlMutationError::Sql(_)
            | crate::data_interface::SqlMutationError::Domain(
                research_agent_core::conversation::CatalogError::Unavailable,
            ) => JournalError::Unavailable,
            _ => JournalError::InvalidInput,
        }),
        EventPayload::TurnStarted { .. } => super::input_queue::require_startable_in(
            db,
            *event.conversation_id().as_uuid(),
            event.request_id(),
        )
        .map_err(|error| match error {
            super::input_queue::Error::Storage => JournalError::Unavailable,
            super::input_queue::Error::Corrupt => JournalError::Corrupt,
            _ => JournalError::InvalidInput,
        }),
        EventPayload::ConversationRolledBack { .. } => {
            super::catalog::recheck_rollback_authority_in(
                db,
                *event.conversation_id().as_uuid(),
                event.request_id(),
            )
            .map_err(|error| match error {
                research_agent_core::conversation::CatalogError::Corrupt => JournalError::Corrupt,
                research_agent_core::conversation::CatalogError::Unavailable => {
                    JournalError::Unavailable
                }
                _ => JournalError::InvalidInput,
            })
        }
        _ => Ok(()),
    }
}

/// Read one consistent committed snapshot. Unknown schema or missing ordinals are
/// errors, not an empty conversation. The caller's decoder determines the item type.
pub fn read_history<Item: DeserializeOwned>(
    connection: &mut Connection,
    query: HistoryQuery,
) -> Result<HistoryPage<Item>, JournalError> {
    let transaction = connection.transaction()?;
    read_history_in(&transaction, query)
}

/// Read another bounded page from a caller-owned consistent snapshot.
pub(crate) fn read_history_in<Item: DeserializeOwned>(
    transaction: &Transaction<'_>,
    query: HistoryQuery,
) -> Result<HistoryPage<Item>, JournalError> {
    let offset = i64::try_from(query.offset).map_err(|_| JournalError::InvalidInput)?;
    let id = query.conversation_id.as_uuid().to_string();
    let count: i64 = transaction
        .query_row(
            "select durable_count from agent_conversations where id = ?1",
            [&id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(JournalError::NotFound)?;
    let mut statement = transaction.prepare(
        "select ordinal, schema_version, payload_json from agent_items
         where conversation_id = ?1 and ordinal >= ?2 order by ordinal limit ?3",
    )?;
    let rows = statement.query_map(params![id, offset, query.limit()], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (ordinal, version, json) = row?;
        if version != 1 || ordinal != offset + items.len() as i64 || ordinal >= count {
            return Err(JournalError::Corrupt);
        }
        items.push(serde_json::from_str(&json).map_err(|_| JournalError::Corrupt)?);
    }
    let expected_len = count
        .saturating_sub(offset)
        .clamp(0, i64::from(query.limit())) as usize;
    if items.len() != expected_len {
        return Err(JournalError::Corrupt);
    }
    let next = query.offset.saturating_add(items.len() as u64);
    Ok(HistoryPage {
        items,
        next_offset: (next < count as u64).then_some(next),
        durable_count: count as u64,
    })
}
