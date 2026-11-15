//! Bounded, immutable bootstrap pages. One SQL transaction freezes every body
//! before any page is returned; subsequent pages never reread a newer message.
use super::*;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAX_RECORDS: usize = 65_536;
const MAX_SNAPSHOTS: usize = 16;
const TTL: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub(crate) enum HistorySnapshotError {
    #[error("agent_history_snapshot_expired")]
    Expired,
    #[error("agent_history_snapshot_too_large")]
    TooLarge,
    #[error("agent_history_snapshot_invalid_cursor")]
    InvalidCursor,
    #[error(transparent)]
    Storage(#[from] StoreError),
}
pub(crate) struct FrozenHistoryPage {
    pub snapshot_id: Uuid,
    pub page: HistoryPage<AgentEvent>,
}
struct Snapshot {
    token: Uuid,
    conversation: ConversationId,
    epoch: Uuid,
    start: u64,
    watermark: u64,
    events: Vec<AgentEvent>,
    bytes: usize,
    created: Instant,
}
#[derive(Default)]
pub(super) struct SnapshotCache {
    snapshots: VecDeque<Snapshot>,
    bytes: usize,
}
impl SnapshotCache {
    fn remove_expired(&mut self, now: Instant) {
        self.snapshots
            .retain(|snapshot| now.duration_since(snapshot.created) < TTL);
        self.bytes = self.snapshots.iter().map(|snapshot| snapshot.bytes).sum();
    }
    fn page(
        &self,
        token: Uuid,
        query: HistoryQuery,
    ) -> Result<FrozenHistoryPage, HistorySnapshotError> {
        let saved = self
            .snapshots
            .iter()
            .find(|snapshot| snapshot.token == token)
            .ok_or(HistorySnapshotError::Expired)?;
        if saved.conversation != query.conversation_id
            || query.offset < saved.start
            || query.offset > saved.watermark
            || saved.epoch.is_nil()
        {
            return Err(HistorySnapshotError::InvalidCursor);
        }
        let first = usize::try_from(query.offset - saved.start)
            .map_err(|_| HistorySnapshotError::InvalidCursor)?;
        let events = saved
            .events
            .iter()
            .skip(first)
            .take(usize::from(query.limit()))
            .cloned()
            .collect::<Vec<_>>();
        let after = query.offset + events.len() as u64;
        Ok(FrozenHistoryPage {
            snapshot_id: token,
            page: HistoryPage {
                items: events,
                durable_count: saved.watermark,
                next_offset: (after < saved.watermark).then_some(after),
            },
        })
    }
    fn insert(&mut self, snapshot: Snapshot) {
        while self.bytes + snapshot.bytes > MAX_BYTES || self.snapshots.len() >= MAX_SNAPSHOTS {
            if let Some(old) = self.snapshots.pop_front() {
                self.bytes -= old.bytes;
            } else {
                break;
            }
        }
        self.bytes += snapshot.bytes;
        self.snapshots.push_back(snapshot);
    }
    fn invalidate(&mut self, conversation: Option<ConversationId>) {
        self.snapshots
            .retain(|snapshot| conversation.is_some_and(|id| snapshot.conversation != id));
        self.bytes = self.snapshots.iter().map(|snapshot| snapshot.bytes).sum();
    }
}

impl AgentSessionService {
    pub(crate) async fn read_frozen_history(
        &self,
        query: HistoryQuery,
        token: Option<Uuid>,
    ) -> Result<Option<FrozenHistoryPage>, HistorySnapshotError> {
        let Some(database) = self.compaction_db.clone().filter(|db| db.logged()) else {
            return if token.is_some() {
                Err(HistorySnapshotError::Expired)
            } else {
                Ok(None)
            };
        };
        let cache = self.history_snapshots.clone();
        tokio::task::spawn_blocking(move || {
            // Deletion invalidates this cache after releasing SQL. Holding it
            // through capture/insert closes the delete-between-read-and-cache race.
            let mut cache = cache.lock().map_err(|_| StoreError::Unavailable)?;
            let now = Instant::now();
            cache.remove_expired(now);
            if let Some(token) = token {
                return cache.page(token, query).map(Some);
            }
            let snapshot = capture(&database, query, now)?;
            let token = snapshot.token;
            cache.insert(snapshot);
            cache.page(token, query).map(Some)
        })
        .await
        .map_err(|_| StoreError::Unavailable)?
    }

    pub(super) fn invalidate_history_snapshots(&self, conversation: Option<ConversationId>) {
        self.history_snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .invalidate(conversation);
    }
}

fn capture(
    database: &crate::data_interface::SharedDatabase,
    query: HistoryQuery,
    created: Instant,
) -> Result<Snapshot, HistorySnapshotError> {
    let mut db = database.lock().map_err(|error| StoreError::Persistence {
        diagnostic: error.to_string(),
    })?;
    let tx = db.transaction().map_err(|_| StoreError::Unavailable)?;
    let epoch: String = tx
        .query_row(
            "select epoch from data_runtime_state where singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| StoreError::Unavailable)?;
    let epoch = Uuid::parse_str(&epoch).map_err(|_| StoreError::CorruptHistory)?;
    let mut events = Vec::new();
    let mut bytes = 0usize;
    let mut offset = query.offset;
    let watermark;
    loop {
        let page = super::super::repository::journal::read_history_in::<AgentEvent>(
            &tx,
            HistoryQuery::new(query.conversation_id, offset, 500)?,
        )
        .map_err(|error| match error {
            super::super::repository::JournalError::NotFound => StoreError::NotFound {
                conversation_id: query.conversation_id,
            },
            super::super::repository::JournalError::Corrupt => StoreError::CorruptHistory,
            _ => StoreError::Unavailable,
        })?;
        if query.offset > page.durable_count {
            return Err(HistorySnapshotError::InvalidCursor);
        }
        for item in page.items {
            bytes = bytes
                .checked_add(
                    serde_json::to_vec(&item)
                        .map_err(|_| StoreError::CorruptHistory)?
                        .len(),
                )
                .ok_or(HistorySnapshotError::TooLarge)?;
            if bytes > MAX_SNAPSHOT_BYTES || events.len() >= MAX_RECORDS {
                return Err(HistorySnapshotError::TooLarge);
            }
            events.push(item);
        }
        if let Some(next) = page.next_offset {
            offset = next;
        } else {
            watermark = page.durable_count;
            break;
        }
    }
    Ok(Snapshot {
        token: Uuid::new_v4(),
        conversation: query.conversation_id,
        epoch,
        start: query.offset,
        watermark,
        events,
        bytes,
        created,
    })
}

#[cfg(test)]
#[path = "history_snapshot/tests.rs"]
mod tests;
