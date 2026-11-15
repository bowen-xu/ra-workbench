#[path = "coordinator_fixture.rs"]
mod fixture;
use fixture::*;
use research_data_runtime::{LogConfig, LogError, RaftLog, coordinator::*};
use std::sync::atomic::Ordering;

fn log_config(path: &std::path::Path) -> LogConfig {
    let mut config = LogConfig::desktop(path);
    config.target_file_bytes = 64 * 1024;
    config.purge_threshold_bytes = 64 * 1024;
    config.rewrite_threshold_bytes = 64 * 1024;
    config.max_managed_file_bytes = 768 * 1024;
    config.maintenance_reserve_bytes = 128 * 1024;
    config
}
fn owner(path: &std::path::Path, fake: &FakeProjector) -> Coordinator<FakeProjector> {
    let epoch = fake.sql.lock().unwrap().epoch;
    let log = RaftLog::open_config(log_config(path), Some(epoch)).unwrap();
    Coordinator::open(
        log,
        fake.clone(),
        CoordinatorConfig {
            materialize_records: 4096,
            reclaim_after_records: 2,
            materialize_bytes: 1024 * 1024,
            max_command_bytes: 1024 * 1024,
            max_pending_bytes: 2 * 1024 * 1024,
            ..config()
        },
    )
    .unwrap()
}
fn large(revision: u64) -> Command {
    let mut value = command(revision, 1);
    let mut state = 0x4d595df4d0f33173_u64 ^ revision;
    value.padding = (0..256 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"[(state % 62) as usize]
                as char
        })
        .collect();
    value
}
#[test]
fn quota_rejection_is_not_accepted_does_not_poison_or_duplicate_and_preserves_large_record() {
    let dir = tempfile::tempdir().unwrap();
    let fake = projector(uuid::Uuid::new_v4());
    let runtime = owner(dir.path(), &fake);
    let first = large(0);
    let saved = runtime
        .write(first.clone(), WriteOptions::default())
        .unwrap();
    assert_eq!(
        runtime
            .write(first, WriteOptions::default())
            .unwrap()
            .operation_id,
        saved.operation_id
    );
    let second = large(1);
    let rejected = runtime
        .write(second.clone(), WriteOptions::default())
        .unwrap_err();
    assert!(
        matches!(rejected,CoordinatorError::NotAccepted {operation_id,position:2,..} if operation_id==second.operation)
    );
    assert_eq!(runtime.accepted_fence().unwrap().position, 1);
    drop(runtime);
    let mut log = RaftLog::open_config(log_config(dir.path()), None).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 1);
    assert_eq!(log.read_range(1, 1, 1024 * 1024).unwrap().len(), 1);
    let usage = log.extent_usage().unwrap();
    assert!(usage.managed_file_extent_bytes + usage.reserved_bytes <= usage.hard_limit_bytes);
    drop(log);
    fake.faults.retention_safe.store(true, Ordering::SeqCst);
    let runtime = owner(dir.path(), &fake);
    let receipt = runtime
        .write(second.clone(), WriteOptions::default())
        .unwrap();
    assert_eq!(receipt.position, 2);
    assert_eq!(
        runtime
            .write(second, WriteOptions::default())
            .unwrap()
            .position,
        2
    );
    assert_eq!(
        fake.faults.allocations.load(Ordering::SeqCst),
        3,
        "a rejected admission can allocate an unobserved result, but each accepted retry is frozen"
    );
}
#[test]
fn query_materialization_reclaims_only_with_explicit_proof_and_repeat_reads_do_not_sync_again() {
    let dir = tempfile::tempdir().unwrap();
    let fake = projector(uuid::Uuid::new_v4());
    let runtime = owner(dir.path(), &fake);
    runtime.write(large(0), WriteOptions::default()).unwrap();
    assert_eq!(
        fake.sql.lock().unwrap().position,
        1,
        "capacity pressure may materialize without granting retention"
    );
    runtime
        .write(command(1, 1), WriteOptions::default())
        .unwrap();
    runtime
        .write(command(2, 1), WriteOptions::default())
        .unwrap();
    runtime
        .write(command(3, 1), WriteOptions::default())
        .unwrap();
    assert_eq!(
        fake.sql.lock().unwrap().position,
        2,
        "the later small suffix remains unmaterialized"
    );
    fake.faults.retention_safe.store(true, Ordering::SeqCst);
    runtime.query(|_, _| Ok(())).unwrap();
    let checks = fake.faults.reclaim_checks.load(Ordering::SeqCst);
    for _ in 0..8 {
        runtime.query(|_, _| Ok(())).unwrap();
    }
    assert_eq!(
        fake.faults.reclaim_checks.load(Ordering::SeqCst),
        checks,
        "unchanged reads must not checkpoint repeatedly"
    );
    let compacted = runtime.accepted_fence().unwrap().position;
    drop(runtime);
    let log = RaftLog::open_config(log_config(dir.path()), None).unwrap();
    assert!(log.head().unwrap().compacted_position > 0);
    assert!(log.head().unwrap().compacted_position <= compacted);
    drop(log);
}
#[test]
fn a_large_record_cannot_use_the_engine_file_slot_estimate_as_its_hard_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = log_config(dir.path());
    config.max_managed_file_bytes = 512 * 1024;
    let mut log = RaftLog::open_config(config, None).unwrap();
    let payload = vec![7; 320 * 1024];
    assert!(matches!(
        log.append(1, &payload, false),
        Err(LogError::CapacityNotAccepted { .. })
    ));
    assert_eq!(log.head().unwrap().accepted_position, 0);
}

#[test]
fn materialized_sql_without_retention_proof_never_compacts_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let fake = projector(uuid::Uuid::new_v4());
    let runtime = open(dir.path(), &fake, config());
    runtime
        .write(command(0, 1), WriteOptions::default())
        .unwrap();
    runtime.query(|_, _| Ok(())).unwrap();
    runtime.shutdown().unwrap();
    drop(runtime);
    let mut log = RaftLog::open(dir.path(), None).unwrap();
    assert_eq!(log.head().unwrap().compacted_position, 0);
    assert_eq!(log.read_range(1, 1, 4096).unwrap().len(), 1);
}

#[test]
fn failure_after_log_acceptance_retains_the_receipt_and_never_appends_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let fake = projector(uuid::Uuid::new_v4());
    fake.faults.reclaim_failure.store(true, Ordering::SeqCst);
    let runtime = owner(dir.path(), &fake);
    let first = large(0);
    assert!(matches!(
        runtime.write(first.clone(), WriteOptions::default()),
        Err(CoordinatorError::AcceptedIncomplete { operation_id, position: 1, .. })
            if operation_id == first.operation
    ));
    assert!(matches!(
        runtime.accepted_fence(),
        Err(CoordinatorError::RecoveryRequired)
    ));
    drop(runtime);
    fake.faults.reclaim_failure.store(false, Ordering::SeqCst);
    let runtime = owner(dir.path(), &fake);
    let restored = runtime.write(first, WriteOptions::default()).unwrap();
    assert!(restored.replayed);
    assert_eq!(restored.position, 1);
    assert_eq!(fake.faults.allocations.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.accepted_fence().unwrap().position, 1);
}
