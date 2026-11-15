use research_data_runtime::RaftLog;
use research_data_runtime::coordinator::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Command {
    pub operation: Uuid,
    pub expected_revision: u64,
    pub expected_external: u64,
    pub delta: i64,
    pub padding: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResultValue {
    pub revision: u64,
    pub total: i64,
    pub allocated_id: Uuid,
}
#[derive(Clone)]
pub struct SqlData {
    pub epoch: Uuid,
    pub position: u64,
    pub revision: u64,
    pub total: i64,
    pub external: u64,
    pub receipts: HashMap<Uuid, OperationReceipt<ResultValue>>,
    pub legacy: HashMap<Uuid, (Command, ResultValue)>,
}
#[derive(Default)]
pub struct Faults {
    pub state: AtomicBool,
    pub lookup: AtomicBool,
    pub before_commit: AtomicBool,
    pub after_commit: AtomicBool,
    pub apply_calls: AtomicUsize,
    pub allocations: AtomicUsize,
    pub retention_safe: AtomicBool,
    pub reclaim_failure: AtomicBool,
    pub reclaim_checks: AtomicUsize,
}
#[derive(Clone)]
pub struct FakeProjector {
    pub sql: Arc<Mutex<SqlData>>,
    pub faults: Arc<Faults>,
}
pub fn failure(message: &'static str) -> io::Error {
    io::Error::other(message)
}
pub fn command(revision: u64, delta: i64) -> Command {
    Command {
        operation: Uuid::new_v4(),
        expected_revision: revision,
        expected_external: 0,
        delta,
        padding: String::new(),
    }
}
pub fn projector(epoch: Uuid) -> FakeProjector {
    FakeProjector {
        sql: Arc::new(Mutex::new(SqlData {
            epoch,
            position: 0,
            revision: 0,
            total: 0,
            external: 0,
            receipts: HashMap::new(),
            legacy: HashMap::new(),
        })),
        faults: Arc::new(Faults::default()),
    }
}
pub fn config() -> CoordinatorConfig {
    CoordinatorConfig {
        materialize_records: 10,
        materialize_bytes: 32 * 1024,
        ..CoordinatorConfig::default()
    }
}
pub fn open(
    path: &Path,
    fake: &FakeProjector,
    config: CoordinatorConfig,
) -> Coordinator<FakeProjector> {
    let epoch = fake.sql.lock().unwrap().epoch;
    let log = RaftLog::open(path, Some(epoch)).unwrap();
    Coordinator::open(log, fake.clone(), config).unwrap()
}
fn apply_one(sql: &mut SqlData, record: &Accepted<Command, ResultValue>) -> Result<(), io::Error> {
    let command = &record.command;
    if record.epoch != sql.epoch
        || record.position != sql.position + 1
        || command.expected_revision != sql.revision
        || command.expected_external != sql.external
        || record.result.revision != sql.revision + 1
        || record.result.total != sql.total + command.delta
        || record.result.allocated_id.is_nil()
        || sql.receipts.contains_key(&record.operation_id)
    {
        return Err(failure("recorded_CAS_or_receipt_mismatch"));
    }
    sql.position = record.position;
    sql.revision += 1;
    sql.total += command.delta;
    sql.receipts.insert(
        record.operation_id,
        OperationReceipt {
            operation_id: record.operation_id,
            position: record.position,
            fingerprint: record.fingerprint,
            result: record.result.clone(),
        },
    );
    Ok(())
}
impl Projector for FakeProjector {
    type Command = Command;
    type Receipt = ResultValue;
    type Error = io::Error;
    fn operation_id(command: &Command) -> Uuid {
        command.operation
    }
    fn state(&self) -> Result<ProjectionState, io::Error> {
        if self.faults.state.load(Ordering::SeqCst) {
            return Err(failure("state_read_failed"));
        }
        let sql = self.sql.lock().unwrap();
        Ok(ProjectionState {
            epoch: sql.epoch,
            materialized_position: sql.position,
        })
    }
    fn lookup(&self, id: Uuid) -> Result<Option<OperationReceipt<ResultValue>>, io::Error> {
        if self.faults.lookup.load(Ordering::SeqCst) {
            return Err(failure("receipt_read_failed"));
        }
        Ok(self.sql.lock().unwrap().receipts.get(&id).cloned())
    }
    fn validate(
        &self,
        command: &Command,
        pending: &[Accepted<Command, ResultValue>],
    ) -> Result<Admission<ResultValue>, io::Error> {
        let mut logical = self.sql.lock().unwrap().clone();
        if let Some((old, receipt)) = logical.legacy.get(&command.operation) {
            return if old == command {
                Ok(Admission::ExistingLegacy(receipt.clone()))
            } else {
                Err(failure("legacy_operation_conflict"))
            };
        }
        for record in pending {
            apply_one(&mut logical, record)?;
        }
        if command.expected_revision != logical.revision
            || command.expected_external != logical.external
        {
            return Err(failure("revision_conflict"));
        }
        self.faults.allocations.fetch_add(1, Ordering::SeqCst);
        Ok(Admission::New(ResultValue {
            revision: logical.revision + 1,
            total: logical.total + command.delta,
            allocated_id: Uuid::new_v4(),
        }))
    }
    fn apply_batch(&self, records: &[Accepted<Command, ResultValue>]) -> Result<(), io::Error> {
        self.faults.apply_calls.fetch_add(1, Ordering::SeqCst);
        let mut sql = self.sql.lock().unwrap();
        let mut transaction = sql.clone();
        for record in records {
            apply_one(&mut transaction, record)?;
        }
        if self.faults.before_commit.load(Ordering::SeqCst) {
            return Err(failure("SQL_transaction_rolled_back"));
        }
        *sql = transaction;
        if self.faults.after_commit.load(Ordering::SeqCst) {
            return Err(failure("SQL_commit_ack_unknown"));
        }
        Ok(())
    }
    fn reclaim_fence(&self, through: ReadFence) -> Result<Option<ReclaimFence>, io::Error> {
        self.faults.reclaim_checks.fetch_add(1, Ordering::SeqCst);
        if self.faults.reclaim_failure.load(Ordering::SeqCst) {
            return Err(failure("reclaim_checkpoint_failed"));
        }
        if !self.faults.retention_safe.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let sql = self.sql.lock().unwrap();
        if sql.epoch != through.epoch || sql.position < through.position {
            return Err(failure("reclaim_proof_ahead_of_SQL"));
        }
        Ok(Some(ReclaimFence {
            epoch: through.epoch,
            through_position: through.position,
            sql_durable_position: through.position,
            retention_safe_position: through.position,
        }))
    }
}
