//! Preserve fields outside the normalized journal as an idempotent projection.
use super::events::{legacy_request_id, legacy_turn_id};
use super::{AppStore, Error, LegacySession, LegacyTurn};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

/// Facts observed in the metadata transaction, not the caller's earlier read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LegacyMetadataSnapshot {
    Pending(String),
    Complete,
    Unmapped,
}

#[derive(Clone, Copy)]
enum WriteScope {
    Refresh,
    PendingRename,
}

/// Keep legacy fields that cannot be represented losslessly by the normalized
/// Agent journal. It is never a model/tool execution source. The pending title
/// freezes only the auxiliary naming intent; completed metadata retains its
/// legacy refresh behavior. Old tables remain until the legacy reader retires.
pub(super) fn persist_legacy_metadata(
    store: &AppStore,
    paper_id: &str,
    legacy: &LegacySession,
    turns: &[LegacyTurn],
    conversation_id: Uuid,
) -> Result<LegacyMetadataSnapshot, Error> {
    persist_metadata(
        store,
        paper_id,
        legacy,
        turns,
        conversation_id,
        WriteScope::Refresh,
    )
}

fn persist_metadata(
    store: &AppStore,
    paper_id: &str,
    legacy: &LegacySession,
    turns: &[LegacyTurn],
    conversation_id: Uuid,
    scope: WriteScope,
) -> Result<LegacyMetadataSnapshot, Error> {
    let mut db = store.connection().map_err(|_| Error::Unavailable)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Unavailable)?;
    let status: Option<String> = tx
        .query_row(
            "select status from agent_paper_reading_history_migrations
         where conversation_id=?1 and paper_id=?2 and legacy_session_id=?3",
            params![conversation_id.to_string(), paper_id, legacy.id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| Error::Unavailable)?;
    if matches!(scope, WriteScope::PendingRename) && status.as_deref() != Some("pending") {
        let snapshot = match status.as_deref() {
            Some("complete") => LegacyMetadataSnapshot::Complete,
            None => LegacyMetadataSnapshot::Unmapped,
            Some(_) => return Err(Error::Corrupt),
        };
        // Do not refresh any auxiliary bytes until the caller verifies the
        // canonical journal for this now-complete mapping.
        tx.commit().map_err(|_| Error::Unavailable)?;
        return Ok(snapshot);
    }
    tx.execute(
        "insert into agent_paper_reading_legacy_sessions(
            conversation_id,paper_id,legacy_session_id,attachment_ids_json,parse_run_ids_json,
            bootstrap_kind,source_workflow_id,source_workflow_revision,bootstrap_messages_json,
            bootstrap_hash,status,title,created_at,updated_at)
         values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         on conflict(conversation_id) do update set
            paper_id=excluded.paper_id,
            legacy_session_id=excluded.legacy_session_id,
            attachment_ids_json=excluded.attachment_ids_json,
            parse_run_ids_json=excluded.parse_run_ids_json,
            bootstrap_kind=excluded.bootstrap_kind,
            source_workflow_id=excluded.source_workflow_id,
            source_workflow_revision=excluded.source_workflow_revision,
            bootstrap_messages_json=excluded.bootstrap_messages_json,
            bootstrap_hash=excluded.bootstrap_hash,
            status=case when exists(
                select 1 from agent_paper_reading_history_migrations m
                where m.conversation_id=excluded.conversation_id
                  and m.paper_id=excluded.paper_id
                  and m.legacy_session_id=excluded.legacy_session_id
                  and m.status='pending'
            ) then agent_paper_reading_legacy_sessions.status else excluded.status end,
            title=case when exists(
                select 1 from agent_paper_reading_history_migrations m
                where m.conversation_id=excluded.conversation_id
                  and m.paper_id=excluded.paper_id
                  and m.legacy_session_id=excluded.legacy_session_id
                  and m.status='pending'
            ) then agent_paper_reading_legacy_sessions.title else excluded.title end,
            created_at=excluded.created_at,
            updated_at=excluded.updated_at",
        params![
            conversation_id.to_string(),
            paper_id,
            legacy.id,
            legacy.attachment_ids_json,
            legacy.parse_run_ids_json,
            legacy.bootstrap_kind,
            legacy.source_workflow_id,
            legacy.source_workflow_revision,
            legacy.bootstrap_messages_json,
            legacy.bootstrap_hash,
            legacy.status,
            legacy.title,
            legacy.created_at,
            legacy.updated_at,
        ],
    )
    .map_err(|_| Error::Unavailable)?;
    for turn in turns {
        tx.execute(
            "insert into agent_paper_reading_legacy_turns(
                conversation_id,legacy_turn_id,request_id,turn_id,ordinal,provider_id,model_id,
                reasoning_effort,status,input_hash,output_hash,input_tokens,output_tokens,
                context_window_tokens,context_window_source,error_code,error_message,messages_json,
                raw_request_json,raw_response_json,created_at,finished_at)
             values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)
             on conflict(conversation_id,legacy_turn_id) do update set
                request_id=excluded.request_id,
                turn_id=excluded.turn_id,
                ordinal=excluded.ordinal,
                provider_id=excluded.provider_id,
                model_id=excluded.model_id,
                reasoning_effort=excluded.reasoning_effort,
                status=excluded.status,
                input_hash=excluded.input_hash,
                output_hash=excluded.output_hash,
                input_tokens=excluded.input_tokens,
                output_tokens=excluded.output_tokens,
                context_window_tokens=excluded.context_window_tokens,
                context_window_source=excluded.context_window_source,
                error_code=excluded.error_code,
                error_message=excluded.error_message,
                messages_json=excluded.messages_json,
                raw_request_json=excluded.raw_request_json,
                raw_response_json=excluded.raw_response_json,
                created_at=excluded.created_at,
                finished_at=excluded.finished_at",
            params![
                conversation_id.to_string(),
                turn.id,
                legacy_request_id(&legacy.id, &turn.id).to_string(),
                legacy_turn_id(&legacy.id, &turn.id).to_string(),
                turn.ordinal,
                turn.provider_id,
                turn.model_id,
                turn.reasoning_effort,
                turn.status,
                turn.input_hash,
                turn.output_hash,
                turn.input_tokens,
                turn.output_tokens,
                turn.context_window_tokens,
                turn.context_window_source,
                turn.error_code,
                turn.error_message,
                turn.messages_json,
                turn.raw_request_json,
                turn.raw_response_json,
                turn.created_at,
                turn.finished_at,
            ],
        )
        .map_err(|_| Error::Unavailable)?;
    }
    let snapshot = match status.as_deref() {
        Some("pending") => LegacyMetadataSnapshot::Pending(
            tx.query_row(
                "select title from agent_paper_reading_legacy_sessions where conversation_id=?1",
                [conversation_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|_| Error::Unavailable)?,
        ),
        Some("complete") => LegacyMetadataSnapshot::Complete,
        None => LegacyMetadataSnapshot::Unmapped,
        Some(_) => return Err(Error::Corrupt),
    };
    tx.commit().map_err(|_| Error::Unavailable)?;
    Ok(snapshot)
}

/// A stale caller must not use an old pending observation to rename or append
/// after another caller completed the mapping. Keep the original journal check.
pub(super) fn prepare_pending_rename_title(
    store: &AppStore,
    paper_id: &str,
    legacy: &LegacySession,
    turns: &[LegacyTurn],
    conversation_id: Uuid,
    payloads: &[research_agent_core::protocol::EventPayload],
) -> Result<Option<String>, Error> {
    match persist_metadata(
        store,
        paper_id,
        legacy,
        turns,
        conversation_id,
        WriteScope::PendingRename,
    )? {
        LegacyMetadataSnapshot::Pending(title) => Ok(Some(title)),
        LegacyMetadataSnapshot::Complete => {
            if !super::journal_matches(store, conversation_id, payloads)? {
                return Err(Error::Conflict);
            }
            Ok(None)
        }
        LegacyMetadataSnapshot::Unmapped => Err(Error::Corrupt),
    }
}
