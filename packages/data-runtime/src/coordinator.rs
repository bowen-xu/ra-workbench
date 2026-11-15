//! One admission owner over a durable log suffix and an atomic SQL prefix.
//! No SQL driver, business command execution, timers, or async runtime lives here.
use crate::{CodecCounters, LogError, Maintenance, RaftLog};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    error::Error,
    sync::{Mutex, MutexGuard},
};
use uuid::Uuid;

#[path = "coordinator_inspection.rs"]
mod inspection;
#[path = "coordinator_maintenance.rs"]
mod maintenance;
#[path = "coordinator_record.rs"]
mod record;
#[path = "coordinator_state.rs"]
mod state;
#[path = "coordinator_transfer.rs"]
mod transfer;
pub use inspection::{InspectionGuard, InspectionSnapshot};

pub type Fingerprint = [u8; 32];
pub type CoordinatorResult<T> = Result<T, CoordinatorError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionState {
    pub epoch: Uuid,
    pub materialized_position: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFence {
    pub epoch: Uuid,
    pub position: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Accepted<C, R> {
    pub epoch: Uuid,
    pub position: u64,
    pub operation_id: Uuid,
    pub fingerprint: Fingerprint,
    pub command: C,
    pub result: R,
}
#[derive(Clone, Debug)]
pub struct OperationReceipt<R> {
    pub operation_id: Uuid,
    pub position: u64,
    pub fingerprint: Fingerprint,
    pub result: R,
}
pub enum Admission<R> {
    New(R),
    ExistingLegacy(R),
}

/// Implementations own business authorization, CAS and frozen result allocation.
/// `validate` reads the SQL base plus the accepted suffix; it must not mutate it.
/// `apply_batch` applies only data, frozen results, operation receipts and the
/// final materialized position in ONE transaction. It must check recorded CAS
/// and never allocate IDs again or invoke a model/tool/external side effect.
/// Error diagnostics must be safe to expose without command bodies or secrets.
pub trait Projector: Send {
    type Command: Serialize + DeserializeOwned + Clone + Send;
    type Receipt: Serialize + DeserializeOwned + Clone + Send;
    type Error: Error + Send + Sync + 'static;
    fn operation_id(command: &Self::Command) -> Uuid;
    fn state(&self) -> Result<ProjectionState, Self::Error>;
    fn lookup(
        &self,
        operation: Uuid,
    ) -> Result<Option<OperationReceipt<Self::Receipt>>, Self::Error>;
    fn validate(
        &self,
        command: &Self::Command,
        pending: &[Accepted<Self::Command, Self::Receipt>],
    ) -> Result<Admission<Self::Receipt>, Self::Error>;
    fn apply_batch(
        &self,
        records: &[Accepted<Self::Command, Self::Receipt>],
    ) -> Result<(), Self::Error>;
    /// Explicit durable-prefix and retention evidence. None forbids automatic
    /// compaction; the runtime never infers permission from materialization.
    fn reclaim_fence(&self, _through: ReadFence) -> Result<Option<ReclaimFence>, Self::Error> {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WriteMode {
    #[default]
    Automatic,
    Immediate,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Durability {
    #[default]
    Process,
    Device,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOptions {
    pub mode: WriteMode,
    pub durability: Durability,
}
#[derive(Clone, Debug)]
pub struct WriteReceipt<R> {
    pub epoch: Uuid,
    /// Log position, or a visibility fence when `legacy` is true. A legacy SQL
    /// transaction does not occupy a new log position.
    pub position: u64,
    pub operation_id: Uuid,
    pub result: R,
    pub materialized: bool,
    pub replayed: bool,
    pub legacy: bool,
}
pub struct QuerySnapshot<T> {
    pub fence: ReadFence,
    pub value: T,
}

/// Retention owner evidence, not a renderer permission. The caller must first
/// establish SQL durability and release backup/content/review references through
/// these positions. Materialization alone is insufficient for safe log deletion.
#[derive(Clone, Copy, Debug)]
pub struct ReclaimFence {
    pub epoch: Uuid,
    pub through_position: u64,
    pub sql_durable_position: u64,
    pub retention_safe_position: u64,
}

#[derive(Clone, Debug)]
pub struct CoordinatorConfig {
    pub materialize_records: usize,
    pub materialize_bytes: usize,
    pub max_pending_records: usize,
    pub max_pending_bytes: usize,
    pub max_command_bytes: usize,
    pub replay_batch_records: usize,
    /// Under pressure, retry an inconclusive reclaim after this many newly
    /// materialized records, without issuing a checkpoint on every query.
    pub reclaim_after_records: usize,
}
impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            materialize_records: 256,
            materialize_bytes: 256 * 1024,
            max_pending_records: 4096,
            max_pending_bytes: 64 * 1024 * 1024,
            max_command_bytes: 32 * 1024 * 1024,
            replay_batch_records: 4096,
            reclaim_after_records: 256,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoordinatorError {
    #[error("data_write_not_accepted: operation_id={operation_id}, position={position}: {cause}")]
    NotAccepted {
        operation_id: Uuid,
        position: u64,
        #[source]
        cause: Box<CoordinatorError>,
    },
    #[error("data_commit_unknown: operation_id={operation_id}, position={position}: {source}")]
    CommitUnknown {
        operation_id: Uuid,
        position: u64,
        #[source]
        source: LogError,
    },
    #[error("data_accepted_incomplete: operation_id={operation_id}, position={position}: {cause}")]
    AcceptedIncomplete {
        operation_id: Uuid,
        position: u64,
        #[source]
        cause: Box<CoordinatorError>,
    },
    #[error(transparent)]
    Log(#[from] LogError),
    #[error("data_invalid_configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("data_epoch_mismatch")]
    EpochMismatch,
    #[error(
        "data_invalid_pairing: accepted={accepted}, materialized={materialized}, compacted={compacted}"
    )]
    InvalidPairing {
        accepted: u64,
        materialized: u64,
        compacted: u64,
    },
    #[error("data_operation_conflict: {operation_id}")]
    OperationConflict { operation_id: Uuid },
    #[error("data_codec_error: position={position}, reason={reason}")]
    Codec { position: u64, reason: &'static str },
    #[error("data_corrupt_record: position={position}, reason={reason}")]
    Corrupt { position: u64, reason: &'static str },
    #[error("data_projection_failed: phase={phase}, position={position}: {source}")]
    Projector {
        phase: &'static str,
        position: u64,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("data_pending_limit: {0}")]
    PendingLimit(&'static str),
    #[error("data_invalid_fence")]
    InvalidFence,
    #[error("data_recovery_required: reopen the data owner before continuing")]
    RecoveryRequired,
    #[error("data_owner_closed")]
    Closed,
}

struct State<P: Projector> {
    log: RaftLog,
    projector: P,
    config: CoordinatorConfig,
    codec_counters: Option<CodecCounters>,
    epoch: Uuid,
    accepted: u64,
    materialized: u64,
    pending: Vec<Accepted<P::Command, P::Receipt>>,
    pending_sizes: Vec<usize>,
    pending_bytes: usize,
    blocked: bool,
    closed: bool,
    last_reclaim_extent: Option<u64>,
    last_reclaim_position: u64,
}
pub struct Coordinator<P: Projector> {
    state: Mutex<State<P>>,
}

impl<P: Projector> Coordinator<P> {
    /// A caller must pair existing directories before opening. SQL owns the
    /// materialized prefix; the retained log suffix is replayed before return.
    pub fn open(log: RaftLog, projector: P, config: CoordinatorConfig) -> CoordinatorResult<Self> {
        Self::open_observed(log, projector, config, None)
    }
    /// Optional per-opening instrumentation covers recovery as well as later
    /// writes; it does not alter validation, codec bytes or durability.
    pub fn open_observed(
        log: RaftLog,
        projector: P,
        config: CoordinatorConfig,
        counters: Option<CodecCounters>,
    ) -> CoordinatorResult<Self> {
        Ok(Self {
            state: Mutex::new(State::recover(log, projector, config, counters)?),
        })
    }
    fn lock(&self) -> CoordinatorResult<MutexGuard<'_, State<P>>> {
        self.state
            .lock()
            .map_err(|_| CoordinatorError::RecoveryRequired)
    }
    pub fn write(
        &self,
        command: P::Command,
        options: WriteOptions,
    ) -> CoordinatorResult<WriteReceipt<P::Receipt>> {
        let mut state = self.lock()?;
        state.ensure_ready()?;
        state.write(command, options)
    }
    pub fn accepted_fence(&self) -> CoordinatorResult<ReadFence> {
        let state = self.lock()?;
        state.ensure_ready()?;
        Ok(state.accepted_fence())
    }
    pub fn materialize(&self, target: ReadFence) -> CoordinatorResult<ReadFence> {
        let mut state = self.lock()?;
        state.ensure_ready()?;
        state.check_fence(target)?;
        state.materialize_through(target.position)?;
        Ok(ReadFence {
            epoch: state.epoch,
            position: state.materialized,
        })
    }
    /// Captures W once and keeps the admission fence while a caller acquires its
    /// SQL connection/transaction. Lock order is this guard, THEN the SQL mutex.
    /// Legacy SQL closures may mutate only tables outside this log's ownership.
    pub fn materialize_guard(&self) -> CoordinatorResult<AdmissionGuard<'_, P>> {
        let mut state = self.lock()?;
        state.ensure_ready()?;
        let fence = state.accepted_fence();
        state.materialize_through(fence.position)?;
        state.maybe_reclaim(false)?;
        Ok(AdmissionGuard { state, fence })
    }
    /// Ordinary query snapshot: >= captured W, not a historical "exact W" SQL
    /// view. Domain absence is `Ok(None)`; a storage query error blocks admission.
    pub fn query<T>(
        &self,
        read: impl FnOnce(&P, ReadFence) -> Result<T, P::Error>,
    ) -> CoordinatorResult<QuerySnapshot<T>> {
        let mut guard = self.materialize_guard()?;
        match read(guard.projector(), guard.fence()) {
            Ok(value) => Ok(QuerySnapshot {
                fence: guard.fence(),
                value,
            }),
            Err(error) => {
                guard.block();
                Err(projector_error("query", guard.fence.position, error))
            }
        }
    }
    /// Uses the low-level nonempty device barrier, never Engine::sync's empty batch.
    pub fn sync(&self) -> CoordinatorResult<ReadFence> {
        let mut state = self.lock()?;
        state.ensure_ready()?;
        let position = state.sync_log()?;
        Ok(ReadFence {
            epoch: state.epoch,
            position,
        })
    }
    pub fn compact_prefix(&self, permission: ReclaimFence) -> CoordinatorResult<()> {
        let mut state = self.lock()?;
        state.ensure_ready()?;
        if permission.epoch != state.epoch {
            return Err(CoordinatorError::EpochMismatch);
        }
        if permission.through_position > state.materialized
            || permission.through_position > permission.sql_durable_position
            || permission.through_position > permission.retention_safe_position
        {
            return Err(CoordinatorError::InvalidFence);
        }
        if let Err(error) = state.log.compact_prefix(permission.through_position, true) {
            state.blocked = true;
            return Err(error.into());
        }
        Ok(())
    }
    pub fn purge(&self) -> CoordinatorResult<Maintenance> {
        let mut state = self.lock()?;
        state.ensure_ready()?;
        match state.log.purge() {
            Ok(result) => Ok(result),
            Err(error) => {
                state.blocked = true;
                Err(error.into())
            }
        }
    }
    /// Stop admission, materialize its fixed prefix and sync once before close.
    /// A failed shutdown remains blocked and must be reopened/reconciled.
    pub fn shutdown(&self) -> CoordinatorResult<ReadFence> {
        let mut state = self.lock()?;
        if state.blocked {
            return Err(CoordinatorError::RecoveryRequired);
        }
        if state.closed {
            return Ok(state.accepted_fence());
        }
        state.closed = true;
        let target = state.accepted;
        state.materialize_through(target)?;
        state.maybe_reclaim(true)?;
        state.sync_log()?;
        Ok(state.accepted_fence())
    }
}

pub struct AdmissionGuard<'a, P: Projector> {
    state: MutexGuard<'a, State<P>>,
    fence: ReadFence,
}
impl<P: Projector> AdmissionGuard<'_, P> {
    pub fn projector(&self) -> &P {
        &self.state.projector
    }
    pub fn fence(&self) -> ReadFence {
        self.fence
    }
    /// A legacy transaction/query with an uncertain result must not allow another
    /// admission over an unconfirmed SQL base. Never clear this in place.
    pub fn block(&mut self) {
        self.state.blocked = true;
    }
}

fn projector_error(
    phase: &'static str,
    position: u64,
    error: impl Error + Send + Sync + 'static,
) -> CoordinatorError {
    CoordinatorError::Projector {
        phase,
        position,
        source: Box::new(error),
    }
}
