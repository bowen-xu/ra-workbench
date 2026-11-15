#[path = "coordinator_fixture.rs"]
mod fixture;
use fixture::*;
use research_data_runtime::{RaftLog, coordinator::*};
use std::sync::atomic::Ordering;

#[test]
fn consuming_shutdown_drains_and_transfers_the_same_locked_engine() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(uuid::Uuid::new_v4());
    let operation = command(0, 7);
    let owner = open(directory.path(), &fake, config());
    let accepted = owner
        .write(operation.clone(), WriteOptions::default())
        .unwrap();
    assert_eq!(fake.sql.lock().unwrap().position, 0);
    let (log, projector) = owner.shutdown_into_parts().unwrap();
    assert_eq!(fake.sql.lock().unwrap().position, 1);
    let head = log.head().unwrap();
    assert_eq!(head.accepted_position, 1);
    assert_eq!(head.synced_position, 1);
    assert!(RaftLog::open(directory.path(), Some(head.epoch)).is_err());
    let owner = Coordinator::open(log, projector, config()).unwrap();
    let replayed = owner.write(operation, WriteOptions::default()).unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.position, accepted.position);
    assert_eq!(replayed.result, accepted.result);
    assert_eq!(fake.faults.allocations.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_consumption_returns_no_parts_and_preserves_the_accepted_suffix() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(uuid::Uuid::new_v4());
    let operation = command(0, 7);
    let owner = open(directory.path(), &fake, config());
    let accepted = owner
        .write(operation.clone(), WriteOptions::default())
        .unwrap();
    fake.faults.before_commit.store(true, Ordering::SeqCst);
    assert!(matches!(
        owner.shutdown_into_parts(),
        Err(CoordinatorError::Projector {
            phase: "apply_batch",
            ..
        })
    ));
    assert_eq!(fake.sql.lock().unwrap().position, 0);
    fake.faults.before_commit.store(false, Ordering::SeqCst);
    let owner = open(directory.path(), &fake, config());
    let replayed = owner.write(operation, WriteOptions::default()).unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.result, accepted.result);
    assert_eq!(replayed.position, 1);
    assert_eq!(fake.faults.allocations.load(Ordering::SeqCst), 1);
}
