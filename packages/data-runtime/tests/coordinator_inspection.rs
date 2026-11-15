#[path = "coordinator_fixture.rs"]
mod fixture;
use fixture::*;
use research_data_runtime::{RaftLog, coordinator::*};
use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};
use uuid::Uuid;

struct ObservedProjector {
    inner: FakeProjector,
    calls: Arc<AtomicUsize>,
}
impl Projector for ObservedProjector {
    type Command = Command;
    type Receipt = ResultValue;
    type Error = io::Error;
    fn operation_id(command: &Command) -> Uuid {
        command.operation
    }
    fn state(&self) -> Result<ProjectionState, io::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.state()
    }
    fn lookup(&self, id: Uuid) -> Result<Option<OperationReceipt<ResultValue>>, io::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.lookup(id)
    }
    fn validate(
        &self,
        command: &Command,
        pending: &[Accepted<Command, ResultValue>],
    ) -> Result<Admission<ResultValue>, io::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.validate(command, pending)
    }
    fn apply_batch(&self, records: &[Accepted<Command, ResultValue>]) -> Result<(), io::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.apply_batch(records)
    }
    fn reclaim_fence(&self, fence: ReadFence) -> Result<Option<ReclaimFence>, io::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.reclaim_fence(fence)
    }
}
fn observed(
    path: &Path,
    fake: &FakeProjector,
    configuration: CoordinatorConfig,
) -> (Coordinator<ObservedProjector>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let epoch = fake.sql.lock().unwrap().epoch;
    let owner = Coordinator::open(
        RaftLog::open(path, Some(epoch)).unwrap(),
        ObservedProjector {
            inner: fake.clone(),
            calls: calls.clone(),
        },
        configuration,
    )
    .unwrap();
    (owner, calls)
}
fn forbid_callbacks(fake: &FakeProjector) {
    fake.faults.state.store(true, Ordering::SeqCst);
    fake.faults.lookup.store(true, Ordering::SeqCst);
    fake.faults.before_commit.store(true, Ordering::SeqCst);
    fake.faults.reclaim_failure.store(true, Ordering::SeqCst);
}

#[test]
fn inspection_keeps_pending_suffix_unmaterialized_and_never_calls_any_projector_callback() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let epoch = fake.sql.lock().unwrap().epoch;
    let (owner, calls) = observed(directory.path(), &fake, config());
    owner.write(command(0, 7), WriteOptions::default()).unwrap();
    let pending_bytes = owner.inspection_guard().unwrap().snapshot().pending_bytes;
    assert!(pending_bytes > 0);
    let before = calls.load(Ordering::SeqCst);
    forbid_callbacks(&fake);
    for _ in 0..3 {
        let guard = owner.inspection_guard().unwrap();
        assert_eq!(
            guard.snapshot(),
            InspectionSnapshot {
                epoch,
                accepted_position: 1,
                materialized_position: 0,
                pending_bytes,
                blocked: false,
                closed: false
            }
        );
        let sql = fake.sql.lock().unwrap(); // Required future status lock order.
        assert_eq!((sql.position, sql.total), (0, 0));
        assert!(sql.receipts.is_empty());
        assert_eq!(guard.snapshot().materialized_position, 0);
    }
    assert_eq!(calls.load(Ordering::SeqCst), before);
    assert_eq!(fake.faults.apply_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fake.faults.reclaim_checks.load(Ordering::SeqCst), 0);
    drop(owner);
    let log = RaftLog::open(directory.path(), Some(epoch)).unwrap();
    let head = log.head().unwrap();
    assert_eq!(
        (
            head.accepted_position,
            head.synced_position,
            head.compacted_position
        ),
        (1, 0, 0)
    );
}

#[test]
fn pending_bytes_match_retained_log_payloads_and_clear_only_after_confirmed_materialization() {
    let directory = tempfile::tempdir().unwrap();
    let epoch = Uuid::new_v4();
    let fake = projector(epoch);
    let owner = open(directory.path(), &fake, config());
    let mut first = command(0, 2);
    first.padding = "escaped \"text\"\n".repeat(17);
    let mut second = command(1, 3);
    second.padding = "正文".repeat(31);
    owner.write(first.clone(), WriteOptions::default()).unwrap();
    owner.write(second, WriteOptions::default()).unwrap();
    let pending = owner.inspection_guard().unwrap().snapshot();
    assert_eq!(
        (pending.accepted_position, pending.materialized_position),
        (2, 0)
    );
    assert!(pending.pending_bytes > first.padding.len());
    owner.write(first, WriteOptions::default()).unwrap();
    assert_eq!(
        owner.inspection_guard().unwrap().snapshot().pending_bytes,
        pending.pending_bytes
    );
    drop(owner);
    let mut log = RaftLog::open(directory.path(), Some(epoch)).unwrap();
    let actual: usize = log
        .read_range(1, 10, 64 * 1024)
        .unwrap()
        .iter()
        .map(|record| record.payload.len())
        .sum();
    assert_eq!(
        pending.pending_bytes, actual,
        "includes the actual persisted record envelope, not just command content"
    );
    let reopened = Coordinator::open(log, fake.clone(), config()).unwrap();
    assert_eq!(
        reopened
            .inspection_guard()
            .unwrap()
            .snapshot()
            .pending_bytes,
        0
    );
    assert_eq!(fake.sql.lock().unwrap().position, 2);
    reopened
        .write(command(2, 5), WriteOptions::default())
        .unwrap();
    assert!(
        reopened
            .inspection_guard()
            .unwrap()
            .snapshot()
            .pending_bytes
            > 0
    );
    reopened.query(|_, _| Ok(())).unwrap();
    assert_eq!(
        reopened
            .inspection_guard()
            .unwrap()
            .snapshot()
            .pending_bytes,
        0
    );
    assert_eq!(fake.sql.lock().unwrap().position, 3);
}

#[test]
fn blocked_inspection_reports_last_confirmed_heads_without_guessing_sql_commit_outcome() {
    for after_commit in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let fake = projector(Uuid::new_v4());
        let (owner, calls) = observed(
            directory.path(),
            &fake,
            CoordinatorConfig {
                materialize_records: 1,
                ..config()
            },
        );
        fake.faults
            .before_commit
            .store(!after_commit, Ordering::SeqCst);
        fake.faults
            .after_commit
            .store(after_commit, Ordering::SeqCst);
        assert!(matches!(
            owner.write(command(0, 7), WriteOptions::default()),
            Err(CoordinatorError::AcceptedIncomplete { .. })
        ));
        let before = calls.load(Ordering::SeqCst);
        forbid_callbacks(&fake);
        let guard = owner.inspection_guard().unwrap();
        let snapshot = guard.snapshot();
        assert!(snapshot.blocked && !snapshot.closed);
        assert!(
            snapshot.pending_bytes > 0,
            "uncertain SQL commit retains its bytes"
        );
        assert_eq!(
            (snapshot.accepted_position, snapshot.materialized_position),
            (1, 0)
        );
        assert_eq!(fake.sql.lock().unwrap().position, u64::from(after_commit));
        drop(guard);
        assert!(matches!(
            owner.write(command(1, 1), WriteOptions::default()),
            Err(CoordinatorError::RecoveryRequired)
        ));
        assert!(matches!(
            owner.query::<()>(|_, _| panic!("blocked inspection cannot enable query")),
            Err(CoordinatorError::RecoveryRequired)
        ));
        assert_eq!(owner.inspection_guard().unwrap().snapshot(), snapshot);
        assert_eq!(calls.load(Ordering::SeqCst), before);
    }
}

#[test]
fn inspection_preserves_explicit_admission_block_and_cannot_clear_it() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let owner = open(directory.path(), &fake, config());
    let mut admission = owner.materialize_guard().unwrap();
    admission.block();
    drop(admission);
    let before = fake.faults.apply_calls.load(Ordering::SeqCst);
    assert!(owner.inspection_guard().unwrap().snapshot().blocked);
    assert!(matches!(
        owner.materialize_guard(),
        Err(CoordinatorError::RecoveryRequired)
    ));
    assert!(matches!(
        owner.write(command(0, 1), WriteOptions::default()),
        Err(CoordinatorError::RecoveryRequired)
    ));
    assert_eq!(fake.faults.apply_calls.load(Ordering::SeqCst), before);
}

#[test]
fn inspection_allows_closed_and_failed_shutdown_states_without_reopening_admission() {
    for failed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let fake = projector(Uuid::new_v4());
        let (owner, calls) = observed(directory.path(), &fake, config());
        owner.write(command(0, 3), WriteOptions::default()).unwrap();
        fake.faults.before_commit.store(failed, Ordering::SeqCst);
        assert_eq!(owner.shutdown().is_err(), failed);
        let before = calls.load(Ordering::SeqCst);
        forbid_callbacks(&fake);
        let snapshot = owner.inspection_guard().unwrap().snapshot();
        assert!(snapshot.closed);
        assert_eq!(snapshot.blocked, failed);
        assert_eq!(snapshot.pending_bytes > 0, failed);
        assert_eq!(
            (snapshot.accepted_position, snapshot.materialized_position),
            (1, u64::from(!failed))
        );
        let write = owner.write(command(1, 2), WriteOptions::default());
        let query = owner.query::<()>(|_, _| panic!("inspection cannot reopen a closed owner"));
        if failed {
            assert!(matches!(write, Err(CoordinatorError::RecoveryRequired)));
            assert!(matches!(query, Err(CoordinatorError::RecoveryRequired)));
        } else {
            assert!(matches!(write, Err(CoordinatorError::Closed)));
            assert!(matches!(query, Err(CoordinatorError::Closed)));
        }
        assert_eq!(calls.load(Ordering::SeqCst), before);
    }
}

#[test]
fn inspection_holds_admission_until_status_sql_guard_has_been_released() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let (owner, calls) = observed(directory.path(), &fake, config());
    let owner = Arc::new(owner);
    owner.write(command(0, 1), WriteOptions::default()).unwrap();
    let inspection = owner.inspection_guard().unwrap();
    let sql = fake.sql.lock().unwrap();
    let before = calls.load(Ordering::SeqCst);
    let (ready, saw_ready) = mpsc::channel();
    let (done, result) = mpsc::channel();
    let worker = owner.clone();
    let writer = std::thread::spawn(move || {
        ready.send(()).unwrap();
        done.send(worker.write(command(1, 2), WriteOptions::default()))
            .unwrap();
    });
    saw_ready
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert_eq!(calls.load(Ordering::SeqCst), before);
    assert_eq!(
        (inspection.snapshot().accepted_position, sql.position),
        (1, 0)
    );
    drop(sql);
    drop(inspection);
    assert_eq!(
        result
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .position,
        2
    );
    writer.join().unwrap();
    let snapshot = owner.inspection_guard().unwrap().snapshot();
    assert_eq!(
        (snapshot.accepted_position, snapshot.materialized_position),
        (2, 0)
    );
}
