//! Idempotent migration of legacy paper-reading chats into the shared Agent journal.
//! This path only replays durable messages; it never calls a model or provider.
use super::{
    repository::SqliteConversationCatalog,
    service::{AgentServiceError, AgentSessionService},
};
use crate::storage::AppStore;
use research_agent_core::{
    conversation::{ConversationCatalog, ConversationId},
    protocol::EventPayload,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;
#[path = "paper_migration/archive.rs"]
mod archive;
#[path = "paper_migration/creation.rs"]
mod creation;
#[path = "paper_migration/error.rs"]
mod error;
#[path = "paper_migration/event_intent.rs"]
mod event_intent;
#[path = "paper_migration/events.rs"]
mod events;
#[path = "paper_migration/metadata.rs"]
mod metadata;
#[path = "paper_migration/rename.rs"]
mod rename;
#[path = "paper_migration/submissions.rs"]
mod submissions;
pub use error::Error;
use events::turn_payloads;
use metadata::{persist_legacy_metadata, prepare_pending_rename_title};
use submissions::prepare_legacy_submissions;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub paper_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub schema_version: u8,
    pub migrated_count: u32,
    pub skipped_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LegacySession {
    id: String,
    title: String,
    status: String,
    context_snapshot_id: Option<String>,
    attachment_ids_json: String,
    parse_run_ids_json: String,
    bootstrap_kind: String,
    source_workflow_id: Option<String>,
    source_workflow_revision: Option<i64>,
    bootstrap_messages_json: String,
    bootstrap_hash: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LegacyTurn {
    id: String,
    ordinal: i64,
    user_message: String,
    assistant_message: String,
    messages_json: String,
    provider_id: String,
    model_id: String,
    reasoning_effort: Option<String>,
    status: String,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    input_hash: Option<String>,
    output_hash: Option<String>,
    context_window_tokens: Option<i64>,
    context_window_source: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
    raw_request_json: Option<String>,
    raw_response_json: Option<String>,
    created_at: String,
    finished_at: Option<String>,
}

#[tauri::command]
pub(crate) async fn agent_migrate_paper_reading_history(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Request,
) -> Result<Response, Error> {
    migrate(&store, &catalog, &sessions, request).await
}

/// Shared by the desktop command and the isolated native preview gateway.
pub(crate) async fn migrate(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    sessions: &Arc<AgentSessionService>,
    request: Request,
) -> Result<Response, Error> {
    let paper_id = request.paper_id.trim().to_owned();
    if paper_id.is_empty() || paper_id.len() > 256 || paper_id.contains('\0') {
        return Err(Error::InvalidInput);
    }
    let legacy = load_legacy(store, &paper_id).map_err(|_| Error::Unavailable)?;
    let mut migrated_count = 0;
    let mut skipped_count = 0;
    for session in legacy {
        match migrate_session(store, catalog, sessions, &paper_id, &session).await? {
            true => migrated_count += 1,
            false => skipped_count += 1,
        }
    }
    Ok(Response {
        schema_version: 1,
        migrated_count,
        skipped_count,
    })
}

fn load_legacy(store: &AppStore, paper_id: &str) -> Result<Vec<LegacySession>, rusqlite::Error> {
    let db = store
        .connection()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    load_legacy_in(&db, paper_id)
}

fn load_legacy_in(
    db: &rusqlite::Connection,
    paper_id: &str,
) -> Result<Vec<LegacySession>, rusqlite::Error> {
    let mut statement = db.prepare(
        "select id, title, status, context_snapshot_id, attachment_ids_json, parse_run_ids_json,
                bootstrap_kind, source_workflow_id, source_workflow_revision,
                bootstrap_messages_json, bootstrap_hash, created_at, updated_at
         from paper_ai_reading_sessions where paper_id = ?1 order by created_at, id",
    )?;
    let rows = statement.query_map([paper_id], |row| {
        Ok(LegacySession {
            id: row.get(0)?,
            title: row.get(1)?,
            status: row.get(2)?,
            context_snapshot_id: row.get(3)?,
            attachment_ids_json: row.get(4)?,
            parse_run_ids_json: row.get(5)?,
            bootstrap_kind: row.get(6)?,
            source_workflow_id: row.get(7)?,
            source_workflow_revision: row.get(8)?,
            bootstrap_messages_json: row.get(9)?,
            bootstrap_hash: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    })?;
    rows.collect()
}

fn load_turns(store: &AppStore, session_id: &str) -> Result<Vec<LegacyTurn>, rusqlite::Error> {
    let db = store
        .connection()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    load_turns_in(&db, session_id)
}

fn load_turns_in(
    db: &rusqlite::Connection,
    session_id: &str,
) -> Result<Vec<LegacyTurn>, rusqlite::Error> {
    let mut statement = db.prepare(
        "select id, ordinal, user_message, assistant_message, messages_json, provider_id, model_id,
                reasoning_effort, status, input_tokens, output_tokens, input_hash, output_hash,
                context_window_tokens, context_window_source, error_code, error_message,
                raw_request_json, raw_response_json, created_at, finished_at
         from paper_ai_reading_turns where session_id = ?1 order by ordinal",
    )?;
    let rows = statement.query_map([session_id], |row| {
        Ok(LegacyTurn {
            id: row.get(0)?,
            ordinal: row.get(1)?,
            user_message: row.get(2)?,
            assistant_message: row.get(3)?,
            messages_json: row.get(4)?,
            provider_id: row.get(5)?,
            model_id: row.get(6)?,
            reasoning_effort: row.get(7)?,
            status: row.get(8)?,
            input_tokens: row.get(9)?,
            output_tokens: row.get(10)?,
            input_hash: row.get(11)?,
            output_hash: row.get(12)?,
            context_window_tokens: row.get(13)?,
            context_window_source: row.get(14)?,
            error_code: row.get(15)?,
            error_message: row.get(16)?,
            raw_request_json: row.get(17)?,
            raw_response_json: row.get(18)?,
            created_at: row.get(19)?,
            finished_at: row.get(20)?,
        })
    })?;
    rows.collect()
}

async fn migrate_session(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    sessions: &AgentSessionService,
    paper_id: &str,
    legacy: &LegacySession,
) -> Result<bool, Error> {
    let turns = load_turns(store, &legacy.id).map_err(|_| Error::Unavailable)?;
    // Do not snapshot a session while its legacy manager can still append a
    // future turn. The next panel mount retries after the old session reaches
    // a terminal state; no dual writer is created.
    let payloads = turns
        .iter()
        .flat_map(|turn| turn_payloads(&legacy.id, turn))
        .collect::<Vec<_>>();
    let expected = payloads.len() as i64;
    let existing = {
        let db = store.connection().map_err(|_| Error::Unavailable)?;
        db.query_row(
            "select conversation_id, expected_event_count, status from agent_paper_reading_history_migrations where paper_id=?1 and legacy_session_id=?2",
            params![paper_id, legacy.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
        ).optional().map_err(|_| Error::Unavailable)?
    };
    if existing.is_none() && legacy.status == "active" {
        return Ok(false);
    }
    if let Some((conversation_id, expected_stored, status)) = existing {
        if expected_stored != expected {
            return Err(Error::Conflict);
        }
        let id = Uuid::parse_str(&conversation_id).map_err(|_| Error::Corrupt)?;
        let frozen = event_intent::validate_if_present(store, paper_id, &legacy.id, id, &payloads)?;
        let (legacy, turns) = match &frozen {
            Some((session, turns)) => (session, turns.as_slice()),
            None => (legacy, turns.as_slice()),
        };
        if status == "complete" {
            if !journal_matches(store, id, &payloads)? {
                return Err(Error::Conflict);
            }
            prepare_legacy_submissions(store, paper_id, legacy, turns, id)?;
            persist_legacy_metadata(store, paper_id, legacy, turns, id)?;
            return Ok(false);
        }
        let summary = catalog
            .read(ConversationId::from_uuid(id), None)
            .await
            .map_err(|_| Error::Unavailable)?;
        let durable = i64::try_from(summary.durable_count).map_err(|_| Error::Corrupt)?;
        if durable == expected_stored {
            let _owner = sessions
                .hold_legacy_finalization(ConversationId::from_uuid(id), summary.durable_count)
                .await
                .map_err(map_session_error)?;
            if !journal_matches(store, id, &payloads)? {
                return Err(Error::Conflict);
            }
            let Some(title) =
                prepare_pending_rename_title(store, paper_id, legacy, turns, id, &payloads)?
            else {
                return Ok(false);
            };
            creation::apply(catalog, id, paper_id).await?;
            rename::apply(catalog, id, &title).await?;
            prepare_legacy_submissions(store, paper_id, legacy, turns, id)?;
            archive::apply(store, catalog, id).await?;
            mark_complete(store, paper_id, &legacy.id, id, expected_stored)
                .map_err(|_| Error::Unavailable)?;
            return Ok(false);
        }
        if durable != 0 || expected_stored == 0 {
            return Err(Error::Conflict);
        }
        let Some(title) =
            prepare_pending_rename_title(store, paper_id, legacy, turns, id, &payloads)?
        else {
            return Ok(false);
        };
        creation::apply(catalog, id, paper_id).await?;
        rename::apply(catalog, id, &title).await?;
        prepare_legacy_submissions(store, paper_id, legacy, turns, id)?;
        sessions
            .open(ConversationId::from_uuid(id))
            .await
            .map_err(map_session_error)?;
        let _owner = sessions
            .commit_fresh_legacy_import(ConversationId::from_uuid(id), &payloads)
            .await
            .map_err(map_session_error)?;
        archive::apply(store, catalog, id).await?;
        mark_complete(store, paper_id, &legacy.id, id, expected_stored)
            .map_err(|_| Error::Unavailable)?;
        return Ok(true);
    }

    let conversation_id = Uuid::new_v4();
    // Claim the durable legacy identity and create its binding in one SQLite
    // transaction. The migration table has a foreign key to the conversation,
    // so claiming before creating the row would always fail on a first import.
    // The transaction still makes the unique legacy identity the concurrency
    // gate and rolls back the provisional conversation on a losing race.
    reserve_mapping(
        store,
        paper_id,
        &legacy.id,
        conversation_id,
        expected,
        Some((legacy, &turns, &payloads)),
    )?;
    creation::apply(catalog, conversation_id, paper_id)
        .await
        .inspect_err(|error| {
            // Only a definite domain rejection can release this provisional claim.
            // A storage failure may have committed; keep its C and phase identity.
            if matches!(
                error,
                Error::Mutation(super::catalog_commands::CreateBoundCommandError::Domain(_))
            ) {
                let _ = release_mapping(store, paper_id, &legacy.id, conversation_id);
            }
        })?;
    let Some(title) =
        prepare_pending_rename_title(store, paper_id, legacy, &turns, conversation_id, &payloads)?
    else {
        return Ok(false);
    };
    rename::apply(catalog, conversation_id, &title).await?;
    prepare_legacy_submissions(store, paper_id, legacy, &turns, conversation_id)?;
    sessions
        .open(ConversationId::from_uuid(conversation_id))
        .await
        .map_err(map_session_error)?;
    let _owner = sessions
        .commit_fresh_legacy_import(ConversationId::from_uuid(conversation_id), &payloads)
        .await
        .map_err(map_session_error)?;
    archive::apply(store, catalog, conversation_id).await?;
    mark_complete(store, paper_id, &legacy.id, conversation_id, expected)
        .map_err(|_| Error::Unavailable)?;
    Ok(true)
}

fn reserve_mapping(
    store: &AppStore,
    paper_id: &str,
    legacy_id: &str,
    conversation_id: Uuid,
    expected: i64,
    original: Option<(&LegacySession, &[LegacyTurn], &[EventPayload])>,
) -> Result<(), Error> {
    let mut db = store.connection().map_err(|_| Error::Unavailable)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Unavailable)?;
    if let Some((session, turns, payloads)) = original {
        if session.id != legacy_id || i64::try_from(payloads.len()).ok() != Some(expected) {
            return Err(Error::Corrupt);
        }
        let derived = turns
            .iter()
            .flat_map(|turn| turn_payloads(legacy_id, turn))
            .collect::<Vec<_>>();
        if derived != payloads {
            return Err(Error::Corrupt);
        }
        let current = load_legacy_in(&tx, paper_id)
            .map_err(|_| Error::Unavailable)?
            .into_iter()
            .find(|row| row.id == legacy_id)
            .ok_or(Error::Conflict)?;
        let current_turns = load_turns_in(&tx, legacy_id).map_err(|_| Error::Unavailable)?;
        if current.status == "active" || &current != session || current_turns != turns {
            return Err(Error::Conflict);
        }
    }
    tx.execute(
        "insert into agent_conversations(id,title) values(?1,'新对话')",
        [conversation_id.to_string()],
    )
    .map_err(|_| Error::Unavailable)?;
    tx.execute(
        "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id)
         values(?1,'paper',?2)",
        params![conversation_id.to_string(), paper_id],
    )
    .map_err(|_| Error::Unavailable)?;
    tx.execute(
        "insert into agent_paper_reading_history_migrations(
            paper_id,legacy_session_id,conversation_id,expected_event_count,status)
         values(?1,?2,?3,?4,'pending')",
        params![paper_id, legacy_id, conversation_id.to_string(), expected],
    )
    .map_err(|_| Error::Unavailable)?;
    if let Some((session, turns, payloads)) = original {
        let changed = tx.execute(
            "update agent_paper_reading_history_migrations set event_intent_required=1 where conversation_id=?1 and paper_id=?2 and legacy_session_id=?3 and status='pending'",
            params![conversation_id.to_string(),paper_id,legacy_id],
        ).map_err(|_| Error::Unavailable)?;
        if changed != 1 {
            return Err(Error::Unavailable);
        }
        event_intent::insert(
            &tx,
            paper_id,
            legacy_id,
            conversation_id,
            session,
            turns,
            payloads,
        )?;
    }
    tx.commit().map_err(|_| Error::Unavailable)
}

fn release_mapping(
    store: &AppStore,
    paper_id: &str,
    legacy_id: &str,
    conversation_id: Uuid,
) -> Result<(), rusqlite::Error> {
    let mut db = store
        .connection()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Keep the claim and binding together if another actor has appended history.
    // An empty count alone is not proof that every possible side effect is absent;
    // this legacy path still requires the complete typed-transaction migration.
    let changed = tx.execute(
        "delete from agent_paper_reading_history_migrations
         where paper_id=?1 and legacy_session_id=?2 and conversation_id=?3 and status='pending'
           and exists(select 1 from agent_conversations where id=?3 and durable_count=0)",
        params![paper_id, legacy_id, conversation_id.to_string()],
    )?;
    if changed == 1 {
        tx.execute(
            "delete from agent_conversation_bindings
             where conversation_id=?1 and resource_kind='paper' and resource_id=?2",
            params![conversation_id.to_string(), paper_id],
        )?;
        tx.execute(
            "delete from agent_conversations
             where id=?1 and durable_count=0
               and not exists(select 1 from agent_conversation_bindings where conversation_id=?1)",
            [conversation_id.to_string()],
        )?;
    }
    tx.commit()
}

fn mark_complete(
    store: &AppStore,
    paper_id: &str,
    legacy_id: &str,
    conversation_id: Uuid,
    expected: i64,
) -> Result<(), rusqlite::Error> {
    let db = store
        .connection()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let changed = db.execute(
        "update agent_paper_reading_history_migrations
         set status='complete', completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
         where paper_id=?1 and legacy_session_id=?2 and conversation_id=?3
           and expected_event_count=?4 and status='pending'
           and exists(select 1 from agent_conversations
                      where id=?3 and durable_count=?4)",
        params![paper_id, legacy_id, conversation_id.to_string(), expected],
    )?;
    if changed == 1 {
        Ok(())
    } else {
        Err(rusqlite::Error::QueryReturnedNoRows)
    }
}

/// Reconcile a migrated legacy session against the immutable shared journal.
/// Counts alone are insufficient: a damaged or partially replaced event with
/// the same ordinal count must not be silently accepted as migrated history.
fn journal_matches(
    store: &AppStore,
    conversation_id: Uuid,
    expected: &[EventPayload],
) -> Result<bool, Error> {
    let db = store.connection().map_err(|_| Error::Unavailable)?;
    let mut statement = db
        .prepare(
            "select ordinal, schema_version, payload_json from agent_items
             where conversation_id=?1 order by ordinal",
        )
        .map_err(|_| Error::Unavailable)?;
    let rows = statement
        .query_map([conversation_id.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| Error::Unavailable)?;
    let mut count = 0usize;
    for row in rows {
        let (ordinal, schema_version, payload_json) = row.map_err(|_| Error::Unavailable)?;
        let Some(expected_payload) = expected.get(count) else {
            return Ok(false);
        };
        if ordinal != i64::try_from(count).map_err(|_| Error::Corrupt)? || schema_version != 1 {
            return Ok(false);
        }
        let envelope_matches =
            serde_json::from_str::<research_agent_core::protocol::AgentEvent>(&payload_json)
                .map(|event| {
                    event.seq() == u64::try_from(count + 1).unwrap_or(u64::MAX)
                        && event.conversation_id() == ConversationId::from_uuid(conversation_id)
                        && event.payload() == expected_payload
                })
                .unwrap_or(false);
        let payload_matches = serde_json::from_str::<EventPayload>(&payload_json)
            .map(|payload| payload == *expected_payload)
            .unwrap_or(false);
        if !envelope_matches && !payload_matches {
            return Ok(false);
        }
        count += 1;
    }
    Ok(count == expected.len())
}

fn map_session_error(error: AgentServiceError) -> Error {
    match error {
        AgentServiceError::Storage(_)
        | AgentServiceError::Session(_)
        | AgentServiceError::Unavailable
        | AgentServiceError::NotOpen
        | AgentServiceError::Capacity
        | AgentServiceError::Active
        | AgentServiceError::LegacyMigrationIncomplete(_)
        | AgentServiceError::RollbackConflict => Error::Unavailable,
    }
}

#[cfg(test)]
#[path = "paper_migration/tests.rs"]
mod tests;
