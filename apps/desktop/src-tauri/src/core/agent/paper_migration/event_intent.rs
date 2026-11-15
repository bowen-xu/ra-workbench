//! Claim-time source and event evidence, not permission to resume a prefix.
use super::{AppStore, Error, EventPayload, LegacySession, LegacyTurn, Uuid};
use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

fn source_hash(session: &str, turns: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"agent-legacy-source-v1\0");
    hash.update((session.len() as u64).to_le_bytes());
    hash.update(session.as_bytes());
    hash.update((turns.len() as u64).to_le_bytes());
    hash.update(turns.as_bytes());
    format!("{:x}", hash.finalize())
}

pub(super) fn insert(
    tx: &Transaction<'_>,
    paper: &str,
    legacy: &str,
    id: Uuid,
    session: &LegacySession,
    turns: &[LegacyTurn],
    payloads: &[EventPayload],
) -> Result<(), Error> {
    let encoded = serde_json::to_string(payloads).map_err(|_| Error::Corrupt)?;
    let session_json = serde_json::to_string(session).map_err(|_| Error::Corrupt)?;
    let turns_json = serde_json::to_string(turns).map_err(|_| Error::Corrupt)?;
    let source_hash = source_hash(&session_json, &turns_json);
    let hash = format!("{:x}", Sha256::digest(encoded.as_bytes()));
    let epoch: String = tx
        .query_row(
            "select epoch from data_runtime_state where singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| Error::Unavailable)?;
    let changed = tx.execute(
        "insert into agent_legacy_event_intents(conversation_id,paper_id,legacy_session_id,epoch,session_json,turns_json,source_sha256,event_count,payloads_json,payloads_sha256) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![id.to_string(), paper, legacy, epoch, session_json, turns_json, source_hash, i64::try_from(payloads.len()).map_err(|_| Error::Corrupt)?, encoded, hash],
    ).map_err(|_| Error::Unavailable)?;
    if changed != 1 {
        return Err(Error::Unavailable);
    }
    Ok(())
}

struct Stored {
    paper: String,
    legacy: String,
    epoch: String,
    session: String,
    turns: String,
    source_hash: String,
    count: i64,
    payloads: String,
    payloads_hash: String,
}

pub(super) fn validate_if_present(
    store: &AppStore,
    paper: &str,
    legacy: &str,
    id: Uuid,
    payloads: &[EventPayload],
) -> Result<Option<(LegacySession, Vec<LegacyTurn>)>, Error> {
    let db = store.connection().map_err(|_| Error::Unavailable)?;
    let stored: Option<Stored> = db.query_row(
        "select paper_id,legacy_session_id,epoch,session_json,turns_json,source_sha256,event_count,payloads_json,payloads_sha256 from agent_legacy_event_intents where conversation_id=?1",
        [id.to_string()], |row| Ok(Stored {paper:row.get(0)?,legacy:row.get(1)?,epoch:row.get(2)?,session:row.get(3)?,turns:row.get(4)?,source_hash:row.get(5)?,count:row.get(6)?,payloads:row.get(7)?,payloads_hash:row.get(8)?}),
    ).optional().map_err(|_| Error::Unavailable)?;
    let required: bool = db.query_row(
        "select event_intent_required from agent_paper_reading_history_migrations where conversation_id=?1 and paper_id=?2 and legacy_session_id=?3",
        params![id.to_string(),paper,legacy], |row| row.get(0),
    ).map_err(|_| Error::Corrupt)?;
    let Some(stored) = stored else {
        return if required {
            Err(Error::Corrupt)
        } else {
            Ok(None)
        };
    };
    if !required {
        return Err(Error::Corrupt);
    }
    let epoch: String = db
        .query_row(
            "select epoch from data_runtime_state where singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| Error::Unavailable)?;
    if stored.paper != paper
        || stored.legacy != legacy
        || stored.epoch != epoch
        || stored.payloads_hash != format!("{:x}", Sha256::digest(stored.payloads.as_bytes()))
        || stored.source_hash != source_hash(&stored.session, &stored.turns)
    {
        return Err(Error::Corrupt);
    }
    let saved: Vec<EventPayload> =
        serde_json::from_str(&stored.payloads).map_err(|_| Error::Corrupt)?;
    let session: LegacySession =
        serde_json::from_str(&stored.session).map_err(|_| Error::Corrupt)?;
    let turns: Vec<LegacyTurn> = serde_json::from_str(&stored.turns).map_err(|_| Error::Corrupt)?;
    let derived = turns
        .iter()
        .flat_map(|turn| super::turn_payloads(&session.id, turn))
        .collect::<Vec<_>>();
    if session.id != legacy
        || session.status == "active"
        || derived != saved
        || i64::try_from(saved.len()).ok() != Some(stored.count)
    {
        return Err(Error::Corrupt);
    }
    if saved != payloads {
        return Err(Error::Conflict);
    }
    Ok(Some((session, turns)))
}
