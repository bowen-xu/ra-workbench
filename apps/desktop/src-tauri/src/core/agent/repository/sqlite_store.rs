use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::conversation::{
    ConversationId, ConversationStore, HistoryPage, HistoryQuery, StoreError, StoreFuture,
    StoreResult, WriterLease,
};
use rusqlite::{Connection, OptionalExtension};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use super::{JournalError, journal, read_history};

/// Async lifecycle adapter over an application-owned SQLite connection. Construct one
/// shared instance per database; never create competing registries for the same database.
/// Blocking jobs hold only owned data and never access frontend or provider state.
pub struct SqliteConversationStore<Item> {
    connection: SharedDatabase,
    writers: Mutex<HashMap<ConversationId, Arc<Writer<Item>>>>,
    initial_title: String,
}

struct Writer<Item> {
    lease: WriterLease,
    active: Mutex<bool>,
    pending: Mutex<Pending<Item>>,
}

struct Pending<Item> {
    state: OpenState,
    items: Vec<Item>,
    count: u64,
    batch_id: Uuid,
    accepted_at_ms: u64,
}

#[derive(Clone, Copy)]
enum OpenState {
    Unopened,
    New,
    Existing,
}

/// Dropping an awaiting future cancels a queued job. Once begin wins under these
/// memory-only gates, the transaction may finish; its durable result is authoritative.
struct Ticket(Arc<Mutex<bool>>);
impl Drop for Ticket {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = true;
    }
}

impl<Item: Clone + Send + Sync + Serialize + DeserializeOwned + 'static>
    SqliteConversationStore<Item>
{
    pub(crate) fn new(connection: impl IntoDatabase, initial_title: String) -> StoreResult<Self> {
        if initial_title.trim().is_empty() || initial_title.chars().count() > 180 {
            return Err(StoreError::InvalidBatch);
        }
        Ok(Self {
            connection: connection.into_database(),
            writers: Mutex::new(HashMap::new()),
            initial_title,
        })
    }

    fn writer(&self, lease: WriterLease) -> StoreResult<Arc<Writer<Item>>> {
        self.writers
            .lock()
            .map_err(|_| StoreError::Unavailable)?
            .get(&lease.conversation_id)
            .filter(|writer| writer.lease == lease)
            .cloned()
            .ok_or(StoreError::WriterConflict)
    }

    async fn run<T: Send + 'static>(
        &self,
        lease: WriterLease,
        operation: impl FnOnce(&mut Connection, &mut Pending<Item>) -> StoreResult<T> + Send + 'static,
    ) -> StoreResult<T> {
        let writer = self.writer(lease)?;
        let connection = Arc::clone(&self.connection);
        let ticket = Ticket(Arc::new(Mutex::new(false)));
        let canceled = Arc::clone(&ticket.0);
        tokio::task::spawn_blocking(move || {
            let mut pending = writer.pending.lock().map_err(|_| StoreError::Unavailable)?;
            let mut connection = connection.lock().map_err(|error| StoreError::Persistence {
                diagnostic: format!(
                    "conversation_id={}: {error}",
                    lease.conversation_id.as_uuid()
                ),
            })?;
            {
                let canceled = canceled.lock().map_err(|_| StoreError::Unavailable)?;
                let active = writer.active.lock().map_err(|_| StoreError::Unavailable)?;
                if *canceled || !*active {
                    return Err(StoreError::WriterConflict);
                }
                // Commit permission linearizes here, before releasing these gates.
                // Discard never waits for SQLite; jobs that already began may finish.
            }
            operation(&mut connection, &mut pending)
        })
        .await
        .map_err(|_| StoreError::Unavailable)?
    }

    async fn save(&self, lease: WriterLease, close: bool) -> StoreResult<u64> {
        if self.connection.logged() {
            return self.save_logged(lease, close).await;
        }
        let title = self.initial_title.clone();
        let writer = self.writer(lease)?;
        self.run(lease, move |connection, pending| {
            let new_title = match pending.state {
                OpenState::Unopened => return Err(StoreError::WriterConflict),
                OpenState::New => Some(title.as_str()),
                OpenState::Existing => None,
            };
            let count = if new_title.is_none() && pending.items.is_empty() {
                pending.count
            } else {
                journal::write_batch(
                    connection,
                    lease.conversation_id,
                    pending.batch_id,
                    pending.count,
                    &pending.items,
                    new_title,
                )
                .map_err(|error| map_error(error, lease.conversation_id))?
            };
            pending.state = OpenState::Existing;
            pending.count = count;
            pending.items.clear();
            pending.batch_id = Uuid::new_v4();
            pending.accepted_at_ms = accepted_at_ms();
            if close {
                // Close while still owning pending, before any queued append can run.
                *writer
                    .active
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = false;
            }
            Ok(count)
        })
        .await
    }

    /// Appending pending items is memory-only. In particular it must not acquire
    /// a query guard, which would materialize the previous streamed fragment.
    async fn run_pending<T: Send + 'static>(
        &self,
        lease: WriterLease,
        operation: impl FnOnce(&mut Pending<Item>) -> StoreResult<T> + Send + 'static,
    ) -> StoreResult<T> {
        let writer = self.writer(lease)?;
        let ticket = Ticket(Arc::new(Mutex::new(false)));
        let canceled = Arc::clone(&ticket.0);
        tokio::task::spawn_blocking(move || {
            let mut pending = writer.pending.lock().map_err(|_| StoreError::Unavailable)?;
            {
                let canceled = canceled.lock().map_err(|_| StoreError::Unavailable)?;
                let active = writer.active.lock().map_err(|_| StoreError::Unavailable)?;
                if *canceled || !*active {
                    return Err(StoreError::WriterConflict);
                }
            }
            operation(&mut pending)
        })
        .await
        .map_err(|_| StoreError::Unavailable)?
    }

    async fn save_logged(&self, lease: WriterLease, close: bool) -> StoreResult<u64> {
        use research_agent_core::protocol::{AgentEvent, EventPayload};
        use research_data_runtime::coordinator::{WriteMode, WriteOptions};
        let title = self.initial_title.clone();
        let database = self.connection.clone();
        let writer = self.writer(lease)?;
        self.run_pending(lease, move |pending| {
            let new_title = match pending.state {
                OpenState::Unopened => return Err(StoreError::WriterConflict),
                OpenState::New => Some(title),
                OpenState::Existing => None,
            };
            if new_title.is_some() || !pending.items.is_empty() {
                // Production stores AgentEvent. Generic legacy fixture stores
                // remain plain SQLite and do not invent a second event codec.
                let events: Vec<AgentEvent> = pending
                    .items
                    .iter()
                    .map(|item| serde_json::to_value(item).and_then(serde_json::from_value))
                    .collect::<Result<_, _>>()
                    .map_err(|_| StoreError::InvalidBatch)?;
                let boundary = new_title.is_some()
                    || events.iter().any(|e| {
                        matches!(
                            e.payload(),
                            EventPayload::TurnStarted { .. }
                                | EventPayload::TurnFinished { .. }
                                | EventPayload::ConversationRolledBack { .. }
                        )
                    });
                let command = crate::data_interface::journal::JournalCommand::new(
                    lease.conversation_id,
                    pending.batch_id,
                    pending.count,
                    events,
                    new_title,
                    pending.accepted_at_ms,
                )
                .map_err(|_| StoreError::InvalidBatch)?;
                let receipt = database
                    .write(
                        command,
                        WriteOptions {
                            mode: if boundary || close {
                                WriteMode::Immediate
                            } else {
                                WriteMode::Automatic
                            },
                            ..Default::default()
                        },
                    )
                    .map_err(|error| StoreError::Persistence {
                        diagnostic: format!(
                            "conversation_id={}, batch_id={}: {error}",
                            lease.conversation_id.as_uuid(),
                            pending.batch_id,
                        ),
                    })?;
                pending.count = receipt.result;
            }
            pending.state = OpenState::Existing;
            pending.items.clear();
            pending.batch_id = Uuid::new_v4();
            pending.accepted_at_ms = accepted_at_ms();
            if close {
                *writer.active.lock().unwrap_or_else(|e| e.into_inner()) = false;
            }
            Ok(pending.count)
        })
        .await
    }
}

impl<Item: Clone + Send + Sync + Serialize + DeserializeOwned + 'static> ConversationStore<Item>
    for SqliteConversationStore<Item>
{
    fn reserve_writer(&self, lease: WriterLease) -> StoreResult<()> {
        let mut writers = self.writers.lock().map_err(|_| StoreError::Unavailable)?;
        if writers.contains_key(&lease.conversation_id) {
            return Err(StoreError::WriterConflict);
        }
        writers.insert(
            lease.conversation_id,
            Arc::new(Writer {
                lease,
                active: Mutex::new(true),
                pending: Mutex::new(Pending {
                    state: OpenState::Unopened,
                    items: vec![],
                    count: 0,
                    batch_id: Uuid::new_v4(),
                    accepted_at_ms: accepted_at_ms(),
                }),
            }),
        );
        Ok(())
    }

    fn create(&self, lease: WriterLease) -> StoreFuture<'_, ()> {
        Box::pin(self.run(lease, move |connection, pending| {
            if !matches!(pending.state, OpenState::Unopened) {
                return Err(StoreError::WriterConflict);
            }
            let exists: bool = connection
                .query_row(
                    "select exists(select 1 from agent_conversations where id = ?1)",
                    [lease.conversation_id.as_uuid().to_string()],
                    |row| row.get(0),
                )
                .map_err(|_| StoreError::Unavailable)?;
            if exists {
                return Err(StoreError::AlreadyExists);
            }
            pending.state = OpenState::New;
            Ok(())
        }))
    }

    fn resume(&self, lease: WriterLease) -> StoreFuture<'_, ()> {
        Box::pin(self.run(lease, move |connection, pending| {
            if !matches!(pending.state, OpenState::Unopened) {
                return Err(StoreError::WriterConflict);
            }
            let count: Option<u64> = connection
                .query_row(
                    "select durable_count from agent_conversations where id = ?1",
                    [lease.conversation_id.as_uuid().to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| StoreError::Unavailable)?;
            pending.count = count.ok_or(StoreError::NotFound {
                conversation_id: lease.conversation_id,
            })?;
            pending.state = OpenState::Existing;
            Ok(())
        }))
    }

    fn append(&self, lease: WriterLease, items: Vec<Item>) -> StoreFuture<'_, ()> {
        Box::pin(self.run_pending(lease, move |pending| {
            if matches!(pending.state, OpenState::Unopened) {
                return Err(StoreError::WriterConflict);
            }
            if pending.items.len().saturating_add(items.len()) > 500 {
                return Err(StoreError::InvalidBatch);
            }
            pending.items.extend(items);
            Ok(())
        }))
    }

    fn persist(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
        Box::pin(self.save(lease, false))
    }
    fn flush(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
        Box::pin(self.save(lease, false))
    }

    fn shutdown(&self, lease: WriterLease) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let count = self.save(lease, true).await?;
            let connection = self.connection.clone();
            tokio::task::spawn_blocking(move || connection.materialize())
                .await
                .map_err(|_| StoreError::Unavailable)?
                .map_err(|error| StoreError::Persistence {
                    diagnostic: error.to_string(),
                })?;
            self.discard(lease);
            Ok(count)
        })
    }

    fn discard(&self, lease: WriterLease) {
        let mut writers = self
            .writers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(writer) = writers
            .get(&lease.conversation_id)
            .filter(|writer| writer.lease == lease)
        {
            *writer
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = false;
            writers.remove(&lease.conversation_id);
        }
    }

    fn history(&self, query: HistoryQuery) -> StoreFuture<'_, HistoryPage<Item>> {
        let connection = Arc::clone(&self.connection);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut connection =
                    connection.lock().map_err(|error| StoreError::Persistence {
                        diagnostic: error.to_string(),
                    })?;
                read_history(&mut connection, query)
                    .map_err(|error| map_error(error, query.conversation_id))
            })
            .await
            .map_err(|_| StoreError::Unavailable)?
        })
    }
}

fn map_error(error: JournalError, conversation_id: ConversationId) -> StoreError {
    match error {
        JournalError::NotFound => StoreError::NotFound { conversation_id },
        JournalError::AlreadyExists => StoreError::AlreadyExists,
        JournalError::RevisionConflict
        | JournalError::BatchConflict
        | JournalError::Archived
        | JournalError::LegacyMigrationIncomplete => StoreError::WriterConflict,
        JournalError::InvalidInput => StoreError::InvalidBatch,
        JournalError::Corrupt => StoreError::CorruptHistory,
        JournalError::Unavailable => StoreError::Unavailable,
    }
}

fn accepted_at_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod commit_race_tests {
    use super::*;
    use research_agent_core::conversation::WriterId;

    #[test]
    fn commit_that_won_before_discard_remains_readable() {
        let mut database = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut database, 67);
        let connection = Arc::new(Mutex::new(database));
        let store = Arc::new(
            SqliteConversationStore::<String>::new(connection.clone(), "Paper".into()).unwrap(),
        );
        let lease = WriterLease {
            conversation_id: ConversationId::from_uuid(Uuid::new_v4()),
            writer_id: WriterId::from_uuid(Uuid::new_v4()),
        };
        store.reserve_writer(lease).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        runtime.block_on(async {
            let mut operation = Box::pin(store.run(lease, move |connection, _| {
                started_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                journal::write_batch(
                    connection,
                    lease.conversation_id,
                    Uuid::new_v4(),
                    0,
                    &["committed"],
                    Some("Paper"),
                )
                .map_err(|error| map_error(error, lease.conversation_id))
            }));
            use std::future::Future;
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(operation.as_mut().poll(&mut context).is_pending());
            // The blocking worker has acquired commit permission. Discard must not
            // wait for its database work, and cannot retroactively erase the commit.
            started_rx.recv().unwrap();
            drop(operation);
            store.discard(lease);
            finish_tx.send(()).unwrap();
        });
        drop(runtime);
        let page = read_history::<String>(
            &mut connection.lock().unwrap(),
            HistoryQuery::new(lease.conversation_id, 0, 10).unwrap(),
        )
        .unwrap();
        assert_eq!(page.items, ["committed"]);
    }
}
