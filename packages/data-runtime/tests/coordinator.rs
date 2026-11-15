#[path = "coordinator_fixture.rs"]
mod fixture;
use fixture::*;
use research_data_runtime::{CodecCounters, RaftLog, coordinator::*};
use std::sync::{Arc, atomic::Ordering, mpsc};
use uuid::Uuid;

#[test]
fn codec_observer_counts_actual_write_replay_and_cold_decode_without_owning_the_log() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let input = command(0, 3);
    let input_bytes = serde_json::to_vec(&input).unwrap().len() as u64;
    let counters = CodecCounters::default();
    let log = RaftLog::open(directory.path(), Some(fake.sql.lock().unwrap().epoch)).unwrap();
    let runtime =
        Coordinator::open_observed(log, fake.clone(), config(), Some(counters.clone())).unwrap();
    assert_eq!(counters.snapshot(), Default::default());
    runtime
        .write(input.clone(), WriteOptions::default())
        .unwrap();
    let first = counters.snapshot();
    assert_eq!(
        (first.command_encodes, first.command_encoded_bytes),
        (2, input_bytes * 2)
    );
    assert_eq!(
        (first.command_round_trips, first.command_round_trip_bytes),
        (1, input_bytes)
    );
    assert_eq!((first.record_encodes, first.record_decodes), (1, 1));
    assert_eq!(first.record_encoded_bytes, first.record_decoded_bytes);
    assert_eq!(
        (first.fingerprints, first.fingerprint_bytes),
        (2, input_bytes * 2)
    );
    runtime.write(input, WriteOptions::default()).unwrap();
    let replay = counters.snapshot();
    assert_eq!(replay.command_encodes, 3);
    assert_eq!(replay.command_round_trips, 2);
    assert_eq!(replay.record_encodes, 1);
    assert_eq!(replay.record_decodes, 1);
    assert_eq!(replay.fingerprints, 3);
    drop(runtime);
    let cold = CodecCounters::default();
    let log = RaftLog::open(directory.path(), Some(fake.sql.lock().unwrap().epoch)).unwrap();
    let reopened =
        Coordinator::open_observed(log, fake.clone(), config(), Some(cold.clone())).unwrap();
    let recovered = cold.snapshot();
    assert_eq!(
        (recovered.record_decodes, recovered.record_decoded_bytes),
        (1, first.record_encoded_bytes)
    );
    assert_eq!((recovered.command_encodes, recovered.fingerprints), (1, 1));
    assert_eq!(
        (recovered.command_round_trips, recovered.record_encodes),
        (0, 0)
    );
    assert_eq!(counters.snapshot(), replay);
    drop(reopened);
}

#[test]
fn automatic_is_durable_before_sql_and_reopen_preserves_frozen_id_and_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let first = command(0, 3);
    let runtime = open(directory.path(), &fake, config());
    let receipt = runtime
        .write(first.clone(), WriteOptions::default())
        .unwrap();
    assert!(!receipt.materialized);
    assert_eq!(fake.sql.lock().unwrap().position, 0);
    let duplicate = runtime
        .write(first.clone(), WriteOptions::default())
        .unwrap();
    assert!(duplicate.replayed);
    assert_eq!(duplicate.result, receipt.result);
    assert_eq!(duplicate.position, 1);
    drop(runtime);
    let restored = open(directory.path(), &fake, config());
    assert_eq!(fake.sql.lock().unwrap().position, 1);
    let duplicate = restored.write(first, WriteOptions::default()).unwrap();
    assert!(duplicate.materialized && duplicate.replayed);
    assert_eq!(duplicate.result.allocated_id, receipt.result.allocated_id);
    assert_eq!(
        fake.faults.allocations.load(Ordering::SeqCst),
        1,
        "replay must not allocate another ID"
    );
}

#[test]
fn thresholds_and_immediate_materialize_whole_prefix_without_changing_durability_mode() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = open(
        directory.path(),
        &fake,
        CoordinatorConfig {
            materialize_records: 2,
            ..config()
        },
    );
    runtime
        .write(command(0, 1), WriteOptions::default())
        .unwrap();
    assert_eq!(fake.sql.lock().unwrap().position, 0);
    runtime
        .write(command(1, 2), WriteOptions::default())
        .unwrap();
    assert_eq!(fake.sql.lock().unwrap().position, 2);
    assert_eq!(fake.faults.apply_calls.load(Ordering::SeqCst), 1);
    runtime
        .write(command(2, 3), WriteOptions::default())
        .unwrap();
    let last = runtime
        .write(
            command(3, 4),
            WriteOptions {
                mode: WriteMode::Immediate,
                durability: Durability::Device,
            },
        )
        .unwrap();
    assert!(last.materialized);
    assert_eq!(fake.sql.lock().unwrap().total, 10);
    drop(runtime);
    let log = RaftLog::open(directory.path(), Some(fake.sql.lock().unwrap().epoch)).unwrap();
    assert_eq!(log.head().unwrap().synced_position, 4);
}

#[test]
fn pending_and_sql_dedup_reject_changed_payload_and_cas_rejection_does_not_poison() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = open(directory.path(), &fake, config());
    let first = command(0, 1);
    runtime
        .write(first.clone(), WriteOptions::default())
        .unwrap();
    let mut changed = first.clone();
    changed.delta = 9;
    assert!(matches!(
        runtime.write(changed.clone(), WriteOptions::default()),
        Err(CoordinatorError::OperationConflict { .. })
    ));
    assert!(
        runtime
            .write(command(0, 2), WriteOptions::default())
            .is_err()
    );
    runtime
        .write(command(1, 2), WriteOptions::default())
        .unwrap();
    runtime
        .materialize(runtime.accepted_fence().unwrap())
        .unwrap();
    assert!(matches!(
        runtime.write(changed, WriteOptions::default()),
        Err(CoordinatorError::OperationConflict { .. })
    ));
    assert_eq!(
        runtime
            .write(first, WriteOptions::default())
            .unwrap()
            .position,
        1
    );
    assert_eq!(
        runtime
            .write(command(2, 4), WriteOptions::default())
            .unwrap()
            .position,
        3
    );
}

#[test]
fn fixed_fence_and_legacy_sql_transaction_exclude_concurrent_admission() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = Arc::new(open(directory.path(), &fake, config()));
    runtime
        .write(command(0, 1), WriteOptions::default())
        .unwrap();
    let guard = runtime.materialize_guard().unwrap();
    assert_eq!(guard.fence().position, 1);
    let (started, entered) = mpsc::channel();
    let (done, result) = mpsc::channel();
    let worker = runtime.clone();
    let thread = std::thread::spawn(move || {
        started.send(()).unwrap();
        let mut next = command(1, 2);
        next.expected_external = 1;
        done.send(worker.write(next, WriteOptions::default()))
            .unwrap();
    });
    entered.recv().unwrap();
    assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
    guard.projector().sql.lock().unwrap().external = 1;
    drop(guard);
    let receipt = result
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    thread.join().unwrap();
    assert_eq!(receipt.position, 2);
    let snapshot = runtime
        .query(|projector, _| Ok(projector.sql.lock().unwrap().total))
        .unwrap();
    assert_eq!((snapshot.fence.position, snapshot.value), (2, 3));
}

#[test]
fn legacy_receipt_has_a_visibility_fence_without_occupying_a_log_position() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let old = command(0, 7);
    let result = ResultValue {
        revision: 9,
        total: 99,
        allocated_id: Uuid::new_v4(),
    };
    fake.sql
        .lock()
        .unwrap()
        .legacy
        .insert(old.operation, (old.clone(), result.clone()));
    let runtime = open(directory.path(), &fake, config());
    let receipt = runtime.write(old, WriteOptions::default()).unwrap();
    assert!(receipt.legacy && receipt.replayed && receipt.materialized);
    assert_eq!(
        (receipt.position, runtime.accepted_fence().unwrap().position),
        (0, 0)
    );
    assert_eq!(receipt.result, result);
    assert_eq!(
        runtime
            .write(command(0, 1), WriteOptions::default())
            .unwrap()
            .position,
        1
    );
}

#[test]
fn query_failure_blocks_further_writes_until_the_owner_is_reopened() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = open(directory.path(), &fake, config());
    runtime
        .write(command(0, 1), WriteOptions::default())
        .unwrap();
    assert!(
        runtime
            .query::<()>(|_, _| Err(failure("query_failed")))
            .is_err()
    );
    assert!(matches!(
        runtime.write(command(1, 1), WriteOptions::default()),
        Err(CoordinatorError::RecoveryRequired)
    ));
    drop(runtime);
    let restored = open(directory.path(), &fake, config());
    assert_eq!(
        restored
            .write(command(1, 1), WriteOptions::default())
            .unwrap()
            .position,
        2
    );
}

#[test]
fn panicking_query_poison_cannot_be_cleared_to_resume_admission_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = open(directory.path(), &fake, config());
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = runtime.query::<()>(|_, _| panic!("synthetic projector panic"));
    }));
    assert!(panic.is_err());
    assert!(matches!(
        runtime.write(command(0, 1), WriteOptions::default()),
        Err(CoordinatorError::RecoveryRequired)
    ));
}

#[test]
fn compaction_requires_materialized_durable_and_retention_safe_prefixes() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = open(directory.path(), &fake, config());
    runtime
        .write(command(0, 1), WriteOptions::default())
        .unwrap();
    let mut permission = ReclaimFence {
        epoch: fake.sql.lock().unwrap().epoch,
        through_position: 1,
        sql_durable_position: 0,
        retention_safe_position: 1,
    };
    assert!(runtime.compact_prefix(permission).is_err());
    runtime
        .materialize(runtime.accepted_fence().unwrap())
        .unwrap();
    assert!(runtime.compact_prefix(permission).is_err());
    permission.sql_durable_position = 1;
    runtime.compact_prefix(permission).unwrap();
    runtime.purge().unwrap();
    drop(runtime);
    let restored = open(directory.path(), &fake, config());
    assert_eq!(
        restored
            .write(command(1, 1), WriteOptions::default())
            .unwrap()
            .position,
        2
    );
}

#[test]
fn pending_pressure_materializes_before_accepting_and_shutdown_rejects_new_writes() {
    let directory = tempfile::tempdir().unwrap();
    let fake = projector(Uuid::new_v4());
    let runtime = open(
        directory.path(),
        &fake,
        CoordinatorConfig {
            materialize_records: 100,
            materialize_bytes: 1900,
            max_pending_records: 100,
            max_pending_bytes: 2000,
            max_command_bytes: 1000,
            replay_batch_records: 100,
            reclaim_after_records: 100,
        },
    );
    let mut oversize = command(0, 1);
    oversize.padding = "x".repeat(1200);
    assert!(matches!(
        runtime.write(oversize, WriteOptions::default()),
        Err(CoordinatorError::PendingLimit(_))
    ));
    for revision in 0..2 {
        let mut next = command(revision, 1);
        next.padding = "x".repeat(700);
        runtime.write(next, WriteOptions::default()).unwrap();
    }
    assert_eq!(fake.sql.lock().unwrap().position, 1);
    assert_eq!(runtime.shutdown().unwrap().position, 2);
    assert_eq!(fake.sql.lock().unwrap().position, 2);
    assert!(matches!(
        runtime.write(command(2, 1), WriteOptions::default()),
        Err(CoordinatorError::Closed)
    ));
    assert_eq!(runtime.shutdown().unwrap().position, 2);
}
