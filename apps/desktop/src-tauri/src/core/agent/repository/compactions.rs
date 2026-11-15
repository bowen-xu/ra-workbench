//! Durable records for bounded model-request history projections.
//! The journal stays authoritative; summaries are explicitly marked as derived.

use research_agent_core::context::compaction::CompactionRecord;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("compaction storage unavailable")]
    Storage,
    #[error("invalid compaction identity or record")]
    Invalid,
}

impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub turn_id: Uuid,
    pub projection: CompactionRecord,
    pub summary_text: Option<String>,
    pub state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Projected,
    Summarized,
    Failed,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Projected => "projected",
            Self::Summarized => "summarized",
            Self::Failed => "failed",
        }
    }
    fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "projected" => Ok(Self::Projected),
            "summarized" => Ok(Self::Summarized),
            "failed" => Ok(Self::Failed),
            _ => Err(Error::Storage),
        }
    }
}

pub fn insert(db: &mut Connection, record: &Record) -> Result<(), Error> {
    validate(record)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("insert into agent_compactions(id,conversation_id,turn_id,history_sha256,original_message_count,retained_message_count,omitted_message_count,original_bytes,retained_bytes,summary_input,summary_text,state) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", params![record.id.to_string(), record.conversation_id.to_string(), record.turn_id.to_string(), record.projection.history_sha256, record.projection.original_message_count as i64, record.projection.retained_message_count as i64, record.projection.omitted_message_count as i64, record.projection.original_bytes as i64, record.projection.retained_bytes as i64, record.projection.summary_input, record.summary_text, record.state.as_str()])?;
    tx.commit()?;
    Ok(())
}

pub fn read(db: &Connection, conversation_id: Uuid, id: Uuid) -> Result<Option<Record>, Error> {
    if conversation_id.is_nil() || id.is_nil() {
        return Err(Error::Invalid);
    }
    db.query_row("select turn_id,history_sha256,original_message_count,retained_message_count,omitted_message_count,original_bytes,retained_bytes,summary_input,summary_text,state from agent_compactions where id=?1 and conversation_id=?2", params![id.to_string(), conversation_id.to_string()], |row| {
        let state = State::parse(&row.get::<_, String>(9)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(Record { id, conversation_id, turn_id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|_| rusqlite::Error::InvalidQuery)?, projection: CompactionRecord { original_message_count: row.get::<_, i64>(2)?.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?, retained_message_count: row.get::<_, i64>(3)?.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?, omitted_message_count: row.get::<_, i64>(4)?.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?, original_bytes: row.get::<_, i64>(5)?.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?, retained_bytes: row.get::<_, i64>(6)?.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?, history_sha256: row.get(1)?, summary_input: row.get(7)? }, summary_text: row.get(8)?, state })
    }).optional().map_err(Into::into)
}

fn validate(record: &Record) -> Result<(), Error> {
    let p = &record.projection;
    if record.id.is_nil()
        || record.conversation_id.is_nil()
        || record.turn_id.is_nil()
        || p.retained_message_count == 0
        || p.history_sha256.len() != 64
        || !p.history_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::Invalid);
    }
    if p.retained_message_count > p.original_message_count
        || p.omitted_message_count + p.retained_message_count > p.original_message_count
    {
        return Err(Error::Invalid);
    }
    if record
        .summary_text
        .as_ref()
        .is_some_and(|text| text.len() > 2 * 1024 * 1024)
    {
        return Err(Error::Invalid);
    }
    match (record.state, record.summary_text.as_deref()) {
        (State::Summarized, Some(text)) if !text.trim().is_empty() => {}
        (State::Projected | State::Failed, None) => {}
        _ => return Err(Error::Invalid),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_migration_tests::migrate_through;

    #[test]
    fn persists_projection_without_promoting_summary_to_history() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate_through(&mut db, 99);
        let conversation = Uuid::new_v4();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'c')",
            [conversation.to_string()],
        )
        .unwrap();
        let record = Record {
            id: Uuid::new_v4(),
            conversation_id: conversation,
            turn_id: Uuid::new_v4(),
            projection: CompactionRecord {
                original_message_count: 4,
                retained_message_count: 2,
                omitted_message_count: 2,
                original_bytes: 40,
                retained_bytes: 20,
                history_sha256: "a".repeat(64),
                summary_input: "old".into(),
            },
            summary_text: Some("derived".into()),
            state: State::Summarized,
        };
        insert(&mut db, &record).unwrap();
        assert_eq!(read(&db, conversation, record.id).unwrap().unwrap(), record);
        assert_eq!(
            db.query_row("select count(*) from agent_items", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn rejects_summary_state_mismatches() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate_through(&mut db, 99);
        let conversation = Uuid::new_v4();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'c')",
            [conversation.to_string()],
        )
        .unwrap();
        let projection = CompactionRecord {
            original_message_count: 4,
            retained_message_count: 2,
            omitted_message_count: 2,
            original_bytes: 40,
            retained_bytes: 20,
            history_sha256: "a".repeat(64),
            summary_input: "old".into(),
        };
        for (state, summary_text) in [
            (State::Summarized, None),
            (State::Summarized, Some("  ".into())),
            (State::Projected, Some("derived".into())),
            (State::Failed, Some("derived".into())),
        ] {
            let record = Record {
                id: Uuid::new_v4(),
                conversation_id: conversation,
                turn_id: Uuid::new_v4(),
                projection: projection.clone(),
                summary_text,
                state,
            };
            assert_eq!(insert(&mut db, &record), Err(Error::Invalid));
        }
    }
}
