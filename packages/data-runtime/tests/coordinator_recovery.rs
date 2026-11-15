#[path = "coordinator_fixture.rs"]
mod fixture;
use fixture::*;
use research_data_runtime::{RaftLog, coordinator::*};
use std::sync::atomic::Ordering;
use uuid::Uuid;

#[test]
fn failed_sql_transaction_keeps_log_and_blocks_admission_then_replays_once() {
    for after_commit in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let fake = projector(Uuid::new_v4());
        fake.faults
            .before_commit
            .store(!after_commit, Ordering::SeqCst);
        fake.faults
            .after_commit
            .store(after_commit, Ordering::SeqCst);
        let runtime = open(
            directory.path(),
            &fake,
            CoordinatorConfig {
                materialize_records: 1,
                ..config()
            },
        );
        let first = command(0, 5);
        assert!(matches!(
            runtime.write(first.clone(), WriteOptions::default()),
            Err(CoordinatorError::AcceptedIncomplete { operation_id, position: 1, .. }) if operation_id == first.operation
        ));
        assert!(matches!(
            runtime.write(command(1, 5), WriteOptions::default()),
            Err(CoordinatorError::RecoveryRequired)
        ));
        assert_eq!(fake.sql.lock().unwrap().position, u64::from(after_commit));
        drop(runtime);
        fake.faults.before_commit.store(false, Ordering::SeqCst);
        fake.faults.after_commit.store(false, Ordering::SeqCst);
        let restored = open(directory.path(), &fake, config());
        let receipt = restored.write(first, WriteOptions::default()).unwrap();
        assert!(receipt.replayed && receipt.materialized);
        assert_eq!((receipt.position, receipt.result.total), (1, 5));
        assert_eq!(fake.sql.lock().unwrap().revision, 1);
        assert_eq!(fake.faults.allocations.load(Ordering::SeqCst), 1);
        assert_eq!(
            fake.faults.apply_calls.load(Ordering::SeqCst),
            if after_commit { 1 } else { 2 }
        );
    }
}

#[test]
fn unknown_record_codec_does_not_skip_forward_or_apply_later_records() {
    let directory = tempfile::tempdir().unwrap();
    let epoch = Uuid::new_v4();
    let fake = projector(epoch);
    let mut log = RaftLog::open(directory.path(), Some(epoch)).unwrap();
    let value = Accepted {
        epoch,
        position: 1,
        operation_id: Uuid::new_v4(),
        fingerprint: [0; 32],
        command: command(0, 1),
        result: ResultValue {
            revision: 1,
            total: 1,
            allocated_id: Uuid::new_v4(),
        },
    };
    let encoded =
        serde_json::to_vec(&serde_json::json!({"schema_version":99,"record":value})).unwrap();
    log.append(1, &encoded, false).unwrap();
    log.append(2, b"not a valid record either", false).unwrap();
    assert!(matches!(
        Coordinator::open(log, fake.clone(), config()),
        Err(CoordinatorError::Codec { position: 1, .. })
    ));
    assert_eq!(fake.sql.lock().unwrap().position, 0);
    assert_eq!(fake.faults.apply_calls.load(Ordering::SeqCst), 0);
    let mut log = RaftLog::open(directory.path(), Some(epoch)).unwrap();
    assert_eq!(log.read_range(1, 8, 65536).unwrap().len(), 2);
}

#[test]
fn recorded_cas_mismatch_in_recovery_retains_suffix_and_does_not_reallocate() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = open(directory.path(), &fake, config());
    runtime
        .write(command(0, 1), WriteOptions::default())
        .unwrap();
    drop(runtime);
    fake.sql.lock().unwrap().revision = 5; // Illegal external write, not a new valid SQL base.
    let log = RaftLog::open(directory.path(), Some(fake.sql.lock().unwrap().epoch)).unwrap();
    assert!(matches!(
        Coordinator::open(log, fake.clone(), config()),
        Err(CoordinatorError::Projector {
            phase: "apply_batch",
            ..
        })
    ));
    assert_eq!(fake.sql.lock().unwrap().position, 0);
    assert_eq!(fake.faults.allocations.load(Ordering::SeqCst), 1);
}

#[test]
fn wrong_epoch_or_missing_sql_prefix_fails_before_admission() {
    let directory = tempfile::tempdir().unwrap();
    let epoch = Uuid::new_v4();
    let log = RaftLog::open(directory.path(), Some(epoch)).unwrap();
    assert!(matches!(
        Coordinator::open(log, projector(Uuid::new_v4()), config()),
        Err(CoordinatorError::EpochMismatch)
    ));
    let fake = projector(epoch);
    fake.sql.lock().unwrap().position = 1;
    let log = RaftLog::open(directory.path(), Some(epoch)).unwrap();
    assert!(matches!(
        Coordinator::open(log, fake, config()),
        Err(CoordinatorError::InvalidPairing { .. })
    ));
}

#[test]
fn bounded_recovery_pages_do_not_depend_on_reexecuting_admission_validation() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = open(directory.path(), &fake, config());
    for revision in 0..7 {
        runtime
            .write(command(revision, 1), WriteOptions::default())
            .unwrap();
    }
    assert_eq!(fake.sql.lock().unwrap().position, 0);
    drop(runtime);
    let restored = open(
        directory.path(),
        &fake,
        CoordinatorConfig {
            replay_batch_records: 2,
            ..config()
        },
    );
    assert_eq!(fake.faults.apply_calls.load(Ordering::SeqCst), 4);
    assert_eq!(fake.sql.lock().unwrap().position, 7);
    assert_eq!(fake.faults.allocations.load(Ordering::SeqCst), 7);
    assert_eq!(restored.accepted_fence().unwrap().position, 7);
}
