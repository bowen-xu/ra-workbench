use std::{
    ops::{Deref, DerefMut},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use research_data_runtime::coordinator::{
    AdmissionGuard, Coordinator, CoordinatorConfig, WriteOptions, WriteReceipt,
};
use rusqlite::Connection;

use super::{
    DataError, bootstrap,
    journal::{JournalCommand, SqlProjector},
};

pub(crate) type SharedDatabase = Arc<Database>;

#[path = "sql_immediate.rs"]
pub(super) mod sql_immediate;

/// The raw SQLite handle is private. Every compatibility SQL operation first
/// materializes a fixed accepted prefix and retains admission through commit.
pub(crate) struct Database {
    raw: Arc<Mutex<Connection>>,
    owner: Option<Coordinator<SqlProjector>>,
    maintenance: Mutex<()>,
}

pub(crate) trait IntoDatabase {
    fn into_database(self) -> SharedDatabase;
}
impl IntoDatabase for SharedDatabase {
    fn into_database(self) -> SharedDatabase {
        self
    }
}
// Plain SQLite is only a compatibility fixture. Production adapters must
// receive the application's paired journal/SQL owner, never an unmanaged one.
#[cfg(test)]
impl IntoDatabase for Arc<Mutex<Connection>> {
    fn into_database(self) -> SharedDatabase {
        Arc::new(Database {
            raw: self,
            owner: None,
            maintenance: Mutex::new(()),
        })
    }
}

pub(crate) struct DatabaseGuard<'a> {
    // Drop SQLite before releasing admission, including transaction rollback.
    raw: MutexGuard<'a, Connection>,
    _admission: Option<AdmissionGuard<'a, SqlProjector>>,
}
impl Deref for DatabaseGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.raw
    }
}
impl DerefMut for DatabaseGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.raw
    }
}
impl DatabaseGuard<'_> {
    pub(crate) fn fence(&self) -> Option<research_data_runtime::ReadFence> {
        self._admission.as_ref().map(AdmissionGuard::fence)
    }
    /// An uncertain compatibility transaction must stop further admission until
    /// the original SQL/log pair is reopened, just like typed SqlImmediate.
    pub(crate) fn require_recovery(&mut self) {
        if let Some(admission) = &mut self._admission {
            admission.block();
        }
    }
}

impl Database {
    #[cfg(test)]
    pub(crate) fn benchmark_snapshot(
        &self,
    ) -> Result<research_data_runtime::InspectionSnapshot, DataError> {
        self.owner
            .as_ref()
            .ok_or(DataError::Unavailable)?
            .inspection_guard()
            .map(|guard| guard.snapshot())
            .map_err(|cause| DataError::Admission(cause.to_string()))
    }
    #[cfg(test)]
    pub(crate) fn open(
        connection: Connection,
        log_path: &Path,
    ) -> Result<SharedDatabase, DataError> {
        Self::open_with_log(connection, log_path, None)
    }

    pub(crate) fn lock_existing_pair(
        connection: &Connection,
        path: &Path,
    ) -> Result<Option<research_data_runtime::RaftLog>, DataError> {
        bootstrap::lock_existing_pair(connection, path)
    }

    pub(crate) fn open_with_log(
        connection: Connection,
        log_path: &Path,
        locked: Option<research_data_runtime::RaftLog>,
    ) -> Result<SharedDatabase, DataError> {
        Self::open_with_log_impl(
            connection,
            log_path,
            locked,
            #[cfg(test)]
            None,
            #[cfg(test)]
            None,
        )
    }
    #[cfg(test)]
    pub(crate) fn benchmark_open_observed(
        connection: Connection,
        log_path: &Path,
        counters: research_data_runtime::LogIoCounters,
        codec: research_data_runtime::CodecCounters,
    ) -> Result<SharedDatabase, DataError> {
        Self::open_with_log_impl(connection, log_path, None, Some(counters), Some(codec))
    }
    fn open_with_log_impl(
        connection: Connection,
        log_path: &Path,
        locked: Option<research_data_runtime::RaftLog>,
        #[cfg(test)] counters: Option<research_data_runtime::LogIoCounters>,
        #[cfg(test)] codec: Option<research_data_runtime::CodecCounters>,
    ) -> Result<SharedDatabase, DataError> {
        super::operation_identity::register(&connection)?;
        // Reclaim evidence below requires FULL SQLite commits, independent of
        // the cheaper process-level durability used for automatic log accepts.
        connection.execute_batch("pragma synchronous=full;")?;
        let raw = Arc::new(Mutex::new(connection));
        #[cfg(test)]
        let log = if let Some(counters) = counters {
            if locked.is_some() {
                return Err(DataError::Pair(
                    "benchmark observation must precede log open",
                ));
            }
            bootstrap::open_pair_observed(&raw, log_path, counters)?
        } else {
            bootstrap::open_pair(&raw, log_path, locked)?
        };
        #[cfg(not(test))]
        let log = bootstrap::open_pair(&raw, log_path, locked)?;
        #[cfg(not(test))]
        let owner = Coordinator::open(
            log,
            SqlProjector::new(Arc::clone(&raw)),
            CoordinatorConfig::default(),
        )
        .map_err(|error| DataError::Admission(error.to_string()))?;
        #[cfg(test)]
        let owner = Coordinator::open_observed(
            log,
            SqlProjector::new(Arc::clone(&raw)),
            CoordinatorConfig::default(),
            codec,
        )
        .map_err(|error| DataError::Admission(error.to_string()))?;
        Ok(Arc::new(Self {
            raw,
            owner: Some(owner),
            maintenance: Mutex::new(()),
        }))
    }

    /// Startup only: replay on the deployed schema through the ordinary owner,
    /// then return its private SQL handle and still-locked engine for migration.
    /// Neither this temporary database nor its guards may have been published.
    pub(crate) fn prepare_schema_migration(
        connection: Connection,
        log_path: &Path,
        locked: research_data_runtime::RaftLog,
    ) -> Result<(Connection, research_data_runtime::RaftLog), DataError> {
        let database = Arc::try_unwrap(Self::open_with_log(connection, log_path, Some(locked))?)
            .map_err(|_| DataError::Pair("startup database was shared before migration"))?;
        let Self { raw, owner, .. } = database;
        let (log, projector) = owner
            .ok_or(DataError::Unavailable)?
            .shutdown_into_parts()
            .map_err(|error| DataError::Admission(error.to_string()))?;
        drop(projector);
        let connection = Arc::try_unwrap(raw)
            .map_err(|_| DataError::Pair("startup SQL handle was shared before migration"))?
            .into_inner()
            .map_err(|_| DataError::Unavailable)?;
        Ok((connection, log))
    }

    pub(crate) fn lock(&self) -> Result<DatabaseGuard<'_>, DataError> {
        let admission = self
            .owner
            .as_ref()
            .map(Coordinator::materialize_guard)
            .transpose()
            .map_err(|error| DataError::Admission(error.to_string()))?;
        let raw = self.raw.lock().map_err(|_| DataError::Unavailable)?;
        Ok(DatabaseGuard {
            raw,
            _admission: admission,
        })
    }

    pub(crate) fn logged(&self) -> bool {
        self.owner.is_some()
    }

    pub(crate) fn write(
        &self,
        command: JournalCommand,
        options: WriteOptions,
    ) -> Result<WriteReceipt<u64>, DataError> {
        self.owner
            .as_ref()
            .ok_or(DataError::Unavailable)?
            .write(command, options)
            .map_err(|error| DataError::Admission(error.to_string()))
    }

    pub(crate) fn materialize(&self) -> Result<(), DataError> {
        drop(self.lock()?);
        Ok(())
    }

    pub(crate) fn shutdown(&self) -> Result<(), DataError> {
        let _maintenance = self
            .maintenance
            .lock()
            .map_err(|_| DataError::Unavailable)?;
        if let Some(owner) = &self.owner {
            owner
                .shutdown()
                .map_err(|e| DataError::Admission(e.to_string()))?;
        }
        Ok(())
    }
}
