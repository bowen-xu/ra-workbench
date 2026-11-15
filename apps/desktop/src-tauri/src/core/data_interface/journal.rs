//! Prepared Agent journal mutations over one SQL prefix. The coordinator owns
//! admission/order; this adapter neither calls models/tools nor allocates replay IDs.
use std::sync::{Arc, Mutex};

use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload},
};
use research_data_runtime::{
    Accepted, Admission, OperationReceipt, ProjectionState, Projector, ReadFence, ReclaimFence,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::repository::journal::{self as storage, JournalError, JournalWrite};

#[path = "journal/coalescer.rs"]
mod coalescer;
use coalescer::BodyCoalescer;

const SCHEMA: u32 = 1;
const MAX_UTC_MS: u64 = 253_402_300_799_999;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalCommand {
    schema_version: u32,
    conversation_id: Uuid,
    batch_id: Uuid,
    expected_count: u64,
    events: Vec<AgentEvent>,
    new_title: Option<String>,
    accepted_at_ms: u64,
}
impl JournalCommand {
    pub(crate) fn new(
        id: ConversationId,
        batch_id: Uuid,
        expected_count: u64,
        events: Vec<AgentEvent>,
        title: Option<String>,
        accepted_at_ms: u64,
    ) -> Result<Self, JournalError> {
        let command = Self {
            schema_version: SCHEMA,
            conversation_id: *id.as_uuid(),
            batch_id,
            expected_count,
            events,
            new_title: title.map(|title| title.trim().to_owned()),
            accepted_at_ms,
        };
        command.check()?;
        Ok(command)
    }
    fn operation(&self) -> Uuid {
        Uuid::new_v5(&self.conversation_id, self.batch_id.as_bytes())
    }
    fn result_count(&self) -> Result<u64, JournalError> {
        self.expected_count
            .checked_add(self.events.len() as u64)
            .filter(|count| i64::try_from(*count).is_ok())
            .ok_or(JournalError::InvalidInput)
    }
    fn check(&self) -> Result<(), JournalError> {
        if self.schema_version != SCHEMA
            || self.conversation_id.is_nil()
            || self.batch_id.is_nil()
            || self.accepted_at_ms > MAX_UTC_MS
            || self.events.len() > 500
            || (self.events.is_empty() && self.new_title.is_none())
            || self.new_title.as_ref().is_some_and(|title| {
                title.trim().is_empty() || title.chars().count() > 180 || self.expected_count != 0
            })
        {
            return Err(JournalError::InvalidInput);
        }
        self.result_count()?;
        for (index, event) in self.events.iter().enumerate() {
            if event.conversation_id().as_uuid() != &self.conversation_id
                || event.seq() != self.expected_count + index as u64 + 1
            {
                return Err(JournalError::InvalidInput);
            }
            if matches!(event.payload(), EventPayload::ConversationRolledBack { .. })
                && (self.events.len() != 1 || self.new_title.is_some())
            {
                return Err(JournalError::InvalidInput);
            }
        }
        Ok(())
    }
}

pub(crate) struct SqlProjector {
    raw: Arc<Mutex<Connection>>,
}
impl SqlProjector {
    pub(crate) fn new(raw: Arc<Mutex<Connection>>) -> Self {
        Self { raw }
    }
}

impl Projector for SqlProjector {
    type Command = JournalCommand;
    type Receipt = u64;
    type Error = JournalError;

    fn operation_id(command: &JournalCommand) -> Uuid {
        command.operation()
    }
    fn state(&self) -> Result<ProjectionState, JournalError> {
        let db = self.raw.lock().map_err(|_| JournalError::Unavailable)?;
        read_state(&db)
    }
    fn lookup(&self, operation: Uuid) -> Result<Option<OperationReceipt<u64>>, JournalError> {
        let db = self.raw.lock().map_err(|_| JournalError::Unavailable)?;
        let state = read_state(&db)?;
        lookup(&db, state, operation)
    }
    fn validate(
        &self,
        command: &JournalCommand,
        pending: &[Accepted<JournalCommand, u64>],
    ) -> Result<Admission<u64>, JournalError> {
        command.check()?;
        let db = self.raw.lock().map_err(|_| JournalError::Unavailable)?;
        read_state(&db)?;
        if super::operation_identity::sql_receipt_exists(&db, command.operation())? {
            return Err(JournalError::BatchConflict);
        }
        let row: Option<(u64, bool)> = db
            .query_row(
                "select durable_count,archived from agent_conversations where id=?1",
                [command.conversation_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        // Keep the deployed lazy-creation rule before legacy receipt lookup.
        if command.new_title.is_some() {
            if row.is_some() {
                return Err(JournalError::AlreadyExists);
            }
            // The permanent-delete trigger is a second guard. Reject before
            // Raft admission so a stale create cannot leave an accepted suffix
            // that SQL will always refuse to replay. SQL deletions share this
            // admission fence, so the proof cannot change before acceptance.
            let deleted: bool = db.query_row(
                "select exists(select 1 from agent_conversation_deletions where conversation_id=?1)",
                [command.conversation_id.to_string()],
                |row| row.get(0),
            )?;
            if deleted {
                return Err(JournalError::NotFound);
            }
        }
        if let Some(result) = legacy_receipt(&db, command)? {
            return Ok(Admission::ExistingLegacy(result));
        }
        // A normal saved submission can race the earlier service-level check.
        // SQL guard changes use the same admission fence, so this check and log
        // admission observe one order. Imported legacy events use other request
        // identities and must remain replayable. Older deployed schemas lack the
        // guard table while their accepted journal is drained before upgrade.
        if command
            .events
            .iter()
            .any(|event| matches!(event.payload(), EventPayload::TurnStarted { .. }))
        {
            let supported: bool = db.query_row(
                "select exists(select 1 from sqlite_schema where type='table' and name='agent_legacy_import_recovery_guards')", [], |row| row.get(0),
            )?;
            if supported {
                for event in &command.events {
                    if let EventPayload::TurnStarted { turn_id, .. } = event.payload() {
                        let protected: bool = db.query_row(
                            "select exists(select 1 from agent_legacy_import_recovery_guards g join agent_submissions s on s.conversation_id=g.conversation_id where g.conversation_id=?1 and s.request_id=?2 and s.turn_id=?3)",
                            params![command.conversation_id.to_string(), event.request_id().to_string(), turn_id.to_string()], |row| row.get(0),
                        )?;
                        if protected {
                            return Err(JournalError::LegacyMigrationIncomplete);
                        }
                    }
                }
            }
        }
        let mut exists = row.is_some();
        let mut count = row.map_or(0, |row| row.0);
        let archived = row.is_some_and(|row| row.1);
        for accepted in pending
            .iter()
            .filter(|record| record.command.conversation_id == command.conversation_id)
        {
            let prior = &accepted.command;
            if prior.new_title.is_some() {
                if exists {
                    return Err(JournalError::Corrupt);
                }
                exists = true;
            }
            if !exists || prior.expected_count != count || prior.result_count()? != accepted.result
            {
                return Err(JournalError::Corrupt);
            }
            count = accepted.result;
        }
        if command.new_title.is_some() {
            if exists {
                return Err(JournalError::AlreadyExists);
            }
        } else if !exists {
            return Err(JournalError::NotFound);
        }
        if archived {
            return Err(JournalError::Archived);
        }
        if command.expected_count != count {
            return Err(JournalError::RevisionConflict);
        }
        validate_hook_prompts(&db, command, pending)?;
        for event in &command.events {
            storage::validate_authority(&db, event)?;
        }
        Ok(Admission::New(command.result_count()?))
    }
    fn apply_batch(&self, records: &[Accepted<JournalCommand, u64>]) -> Result<(), JournalError> {
        if records.is_empty() {
            return Ok(());
        }
        let mut db = self.raw.lock().map_err(|_| JournalError::Unavailable)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = read_state(&tx)?;
        let mut position = state.materialized_position;
        let enabled: bool = tx.query_row(
            "select coalesce_text from data_projection_control where singleton=1",
            [],
            |row| row.get(0),
        )?;
        if enabled {
            return Err(JournalError::Corrupt);
        }
        tx.execute(
            "update data_projection_control set coalesce_text=1 where singleton=1",
            [],
        )?;
        let mut bodies = BodyCoalescer::default();
        for record in records {
            let command = &record.command;
            command.check()?;
            if record.epoch != state.epoch
                || record.operation_id != command.operation()
                || command.result_count()? != record.result
            {
                return Err(JournalError::Corrupt);
            }
            if record.position <= position {
                let saved = lookup(
                    &tx,
                    ProjectionState {
                        epoch: state.epoch,
                        materialized_position: position,
                    },
                    record.operation_id,
                )?
                .ok_or(JournalError::Corrupt)?;
                if saved.position != record.position
                    || saved.fingerprint != record.fingerprint
                    || saved.result != record.result
                {
                    return Err(JournalError::Corrupt);
                }
                continue;
            }
            if record.position != position + 1 {
                return Err(JournalError::Corrupt);
            }
            let at: String =
                tx.query_row(
                    "select strftime('%Y-%m-%dT%H:%M:%fZ',?1 / 1000.0,'unixepoch')",
                    [i64::try_from(command.accepted_at_ms)
                        .map_err(|_| JournalError::InvalidInput)?],
                    |row| row.get(0),
                )?;
            if command.events.iter().any(|event| {
                matches!(
                    event.payload(),
                    EventPayload::ItemAccepted {
                        kind: research_agent_core::protocol::ItemKind::HookPrompt { .. },
                        ..
                    }
                )
            }) {
                // Read the just-projected prefix only after its coalesced bodies
                // are materialized, while keeping ordinary delta batching intact.
                bodies.flush(&tx)?;
                validate_hook_prompts(&tx, command, &[])?;
            }
            let result = storage::write_batch_in_tx(
                &tx,
                JournalWrite {
                    id: ConversationId::from_uuid(command.conversation_id),
                    batch_id: command.batch_id,
                    expected_count: command.expected_count,
                    items: &command.events,
                    new_title: command.new_title.as_deref(),
                    frozen_at: Some(&at),
                },
                |tx, ordinal, event| bodies.before_insert(tx, ordinal, event),
            )?;
            if result != record.result {
                return Err(JournalError::Corrupt);
            }
            tx.execute("insert into data_operation_receipts(operation_id,epoch,position,fingerprint,result_json) values(?1,?2,?3,?4,?5)",
                params![record.operation_id.to_string(),state.epoch.to_string(),i64::try_from(record.position).map_err(|_|JournalError::Corrupt)?,
                    record.fingerprint.as_slice(),serde_json::to_string(&record.result).map_err(|_|JournalError::Corrupt)?])?;
            position = record.position;
        }
        bodies.flush(&tx)?;
        tx.execute(
            "update data_projection_control set coalesce_text=0 where singleton=1",
            [],
        )?;
        let changed = tx.execute("update data_runtime_state set materialized_position=?1 where singleton=1 and schema_version=1 and epoch=?2 and bootstrap_state='ready'",
            params![i64::try_from(position).map_err(|_| JournalError::Corrupt)?,state.epoch.to_string()])?;
        if changed != 1 {
            return Err(JournalError::Corrupt);
        }
        tx.commit()?;
        Ok(())
    }
    fn reclaim_fence(&self, through: ReadFence) -> Result<Option<ReclaimFence>, JournalError> {
        let db = self.raw.lock().map_err(|_| JournalError::Unavailable)?;
        let state = read_state(&db)?;
        if state.epoch != through.epoch || state.materialized_position < through.position {
            return Err(JournalError::Corrupt);
        }
        let synchronous: u32 = db.pragma_query_value(None, "synchronous", |row| row.get(0))?;
        if synchronous < 2 {
            return Err(JournalError::Corrupt);
        }
        let (busy, frames, checkpointed): (u32, i64, i64) =
            db.query_row("pragma wal_checkpoint(full)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        if busy != 0 || frames >= 0 && checkpointed < frames {
            return Ok(None);
        }
        // This projector accepts only data-only Agent journal commands. Their
        // entire history, native extensions and idempotent receipts are in the
        // FULL SQL prefix. Current backups own copies; no consumer retains a
        // log-only prefix. New command families must prove their own retention.
        Ok(Some(ReclaimFence {
            epoch: through.epoch,
            through_position: through.position,
            sql_durable_position: through.position,
            retention_safe_position: through.position,
        }))
    }
}

/// Source validation happens before Raft admission; rejected prompts must not
/// create a suffix that the SQL projector cannot safely materialize.
fn validate_hook_prompts(
    db: &Connection,
    command: &JournalCommand,
    pending: &[Accepted<JournalCommand, u64>],
) -> Result<(), JournalError> {
    use research_agent_core::protocol::ItemKind;
    let is_hook = |event: &AgentEvent| {
        matches!(
            event.payload(),
            EventPayload::ItemAccepted {
                kind: ItemKind::HookPrompt { .. },
                ..
            }
        )
    };
    if !command.events.iter().any(is_hook) {
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (db, pending);
        return Err(JournalError::InvalidInput);
    }
    #[cfg(target_os = "macos")]
    {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction()?;
            validate_hook_prompts_in_snapshot(&tx, command, pending)
        } else {
            validate_hook_prompts_in_snapshot(db, command, pending)
        }
    }
}

#[cfg(target_os = "macos")]
fn validate_hook_prompts_in_snapshot(
    db: &Connection,
    command: &JournalCommand,
    pending: &[Accepted<JournalCommand, u64>],
) -> Result<(), JournalError> {
    use research_agent_core::protocol::ItemKind;
    let mut history =
        crate::agent::repository::context_windows::read_projection(db, command.conversation_id)
            .map_err(|_| JournalError::Corrupt)?;
    let mut pending_invocations = std::collections::HashSet::new();
    let mut model_starts = Vec::new();
    for event in pending
        .iter()
        .filter(|record| record.command.conversation_id == command.conversation_id)
        .flat_map(|record| record.command.events.iter())
    {
        if let EventPayload::ItemAccepted {
            kind: ItemKind::HookPrompt { invocation_id, .. },
            ..
        } = event.payload()
            && !pending_invocations.insert(*invocation_id)
        {
            return Err(JournalError::Corrupt);
        }
        if let EventPayload::ModelRequestStarted { turn_id, .. } = event.payload() {
            model_starts.push((*turn_id, event.seq()));
        }
        history.apply(event).map_err(|_| JournalError::Corrupt)?;
    }
    if history.seq() != command.expected_count {
        return Err(JournalError::Corrupt);
    }
    for event in &command.events {
        if let EventPayload::ItemAccepted {
            kind: ItemKind::HookPrompt { invocation_id, .. },
            ..
        } = event.payload()
        {
            if !pending_invocations.insert(*invocation_id) {
                return Err(JournalError::InvalidInput);
            }
            crate::agent::repository::stop_hook_audit::validate_prompt_source(
                db,
                event,
                &history,
                &model_starts,
            )?;
        }
        if let EventPayload::ModelRequestStarted { turn_id, .. } = event.payload() {
            model_starts.push((*turn_id, event.seq()));
        }
        history
            .apply(event)
            .map_err(|_| JournalError::InvalidInput)?;
    }
    Ok(())
}

fn read_state(db: &Connection) -> Result<ProjectionState, JournalError> {
    let (schema, epoch, position, bootstrap): (u32,String,u64,String) = db.query_row(
        "select schema_version,epoch,materialized_position,bootstrap_state from data_runtime_state where singleton=1",
        [], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
    let epoch = Uuid::parse_str(&epoch).map_err(|_| JournalError::Corrupt)?;
    if schema != SCHEMA || epoch.is_nil() || bootstrap != "ready" {
        return Err(JournalError::Corrupt);
    }
    Ok(ProjectionState {
        epoch,
        materialized_position: position,
    })
}

fn lookup(
    db: &Connection,
    state: ProjectionState,
    operation: Uuid,
) -> Result<Option<OperationReceipt<u64>>, JournalError> {
    let row: Option<(String,u64,Vec<u8>,String)> = db.query_row(
        "select epoch,position,fingerprint,result_json from data_operation_receipts where operation_id=?1",
        [operation.to_string()], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    row.map(|(epoch, position, fingerprint, result)| {
        if epoch != state.epoch.to_string()
            || position == 0
            || position > state.materialized_position
        {
            return Err(JournalError::Corrupt);
        }
        Ok(OperationReceipt {
            operation_id: operation,
            position,
            fingerprint: fingerprint.try_into().map_err(|_| JournalError::Corrupt)?,
            result: serde_json::from_str(&result).map_err(|_| JournalError::Corrupt)?,
        })
    })
    .transpose()
}

fn legacy_receipt(db: &Connection, command: &JournalCommand) -> Result<Option<u64>, JournalError> {
    let row: Option<(u64,u64,String)> = db.query_row(
        "select expected_count,result_count,payload_hash from agent_journal_batches where conversation_id=?1 and batch_id=?2",
        params![command.conversation_id.to_string(),command.batch_id.to_string()],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    row.map(|(expected, result, hash)| {
        if expected != command.expected_count
            || result != command.result_count()?
            || hash != storage::payload_hash(&command.events)?
        {
            return Err(JournalError::BatchConflict);
        }
        Ok(result)
    })
    .transpose()
}

#[cfg(test)]
#[path = "journal/tests.rs"]
mod tests;
