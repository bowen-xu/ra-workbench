use super::*;
use protobuf::Message;

mod faults;
mod preparing;

fn append(log: &mut RaftLog, count: u64) {
    for position in 1..=count {
        log.append(position, format!("mutation-{position}").as_bytes(), false)
            .unwrap();
    }
}

#[test]
fn serial_unsynced_writes_reopen_before_a_desktop_file_fills() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("log");
    let epoch = Uuid::new_v4();
    let mut log = RaftLog::open(&path, Some(epoch)).unwrap();
    append(&mut log, 41);
    assert_eq!(log.head().unwrap().synced_position, 0);
    let footprint = log.purge().unwrap();
    println!("desktop log: {footprint:?}");
    assert!(footprint.bytes_on_disk < 4 * 1024 * 1024);
    drop(log);
    let mut log = RaftLog::open(&path, Some(epoch)).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 41);
    let records = log.read_range(1, 100, 4096).unwrap();
    assert_eq!(records.len(), 41);
    assert_eq!(records[40].payload, b"mutation-41");
    log.append(42, b"after reopen", false).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 42);
}

#[test]
fn bounded_reads_report_an_oversized_first_record_and_never_skip_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RaftLog::open(dir.path(), None).unwrap();
    log.append(1, b"large-record", false).unwrap();
    log.append(2, b"x", false).unwrap();
    assert!(matches!(
        log.read_range(1, 2, 2),
        Err(LogError::RecordTooLarge)
    ));
    assert_eq!(log.read_range(1, 1, 100).unwrap().len(), 1);
    assert_eq!(log.read_range(2, 2, 1).unwrap()[0].payload, b"x");
    assert!(log.read_range(3, 2, 1).unwrap().is_empty());
}

#[test]
fn compact_only_deletes_the_attested_prefix_and_never_resets_global_positions() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RaftLog::open(dir.path(), None).unwrap();
    append(&mut log, 6);
    log.compact_prefix(2, false).unwrap();
    assert!(matches!(
        log.read_range(1, 1, 100),
        Err(LogError::Compacted)
    ));
    assert_eq!(
        log.read_range(3, 10, 100)
            .unwrap()
            .iter()
            .map(|record| record.position)
            .collect::<Vec<_>>(),
        [3, 4, 5, 6]
    );
    assert!(matches!(
        log.compact_prefix(7, false),
        Err(LogError::InvalidArgument(_))
    ));
    log.sync().unwrap();
    drop(log);
    let mut log = RaftLog::open(dir.path(), None).unwrap();
    assert_eq!(log.head().unwrap().compacted_position, 2);
    log.compact_prefix(6, true).unwrap();
    drop(log);
    let mut log = RaftLog::open(dir.path(), None).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 6);
    assert_eq!(log.head().unwrap().compacted_position, 6);
    log.append(7, b"still monotonic", false).unwrap();
    assert_eq!(log.read_range(7, 1, 100).unwrap()[0].position, 7);
}

#[test]
fn epoch_mismatch_or_nonempty_log_without_metadata_cannot_initialize_a_fresh_store() {
    let dir = tempfile::tempdir().unwrap();
    let first = Uuid::new_v4();
    drop(RaftLog::open(dir.path(), Some(first)).unwrap());
    let error = match RaftLog::open(dir.path(), Some(Uuid::new_v4())) {
        Err(error) => error,
        Ok(_) => panic!("another epoch must not open"),
    };
    assert!(matches!(error, LogError::EpochMismatch), "{error:?}");
    let other = tempfile::tempdir().unwrap();
    let engine = Engine::open(LogConfig::desktop(other.path()).engine_config().unwrap()).unwrap();
    let mut batch = LogBatch::default();
    batch
        .put(1, b"foreign".to_vec(), b"existing content".to_vec())
        .unwrap();
    engine.write(&mut batch, true).unwrap();
    drop(engine);
    assert!(matches!(
        RaftLog::open(other.path(), None),
        Err(LogError::Corrupt(_))
    ));
}

#[test]
fn protobuf_envelope_is_versioned_and_unknown_schema_poisoning_prevents_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RaftLog::open(dir.path(), None).unwrap();
    log.append(1, &[0, 255, 0, 17], false).unwrap();
    let mut envelope = log
        .engine
        .get_entry::<EnvelopeIndex>(GROUP, 1)
        .unwrap()
        .unwrap();
    let encoded = envelope.write_to_bytes().unwrap();
    assert_eq!(LogEnvelope::parse_from_bytes(&encoded).unwrap(), envelope);
    envelope.schema_version += 1;
    let mut batch = LogBatch::default();
    batch
        .add_entries::<EnvelopeIndex>(GROUP, &[envelope])
        .unwrap();
    log.engine.write(&mut batch, false).unwrap();
    assert!(matches!(
        log.read_range(1, 1, 100),
        Err(LogError::Corrupt(_))
    ));
    assert!(matches!(log.head(), Err(LogError::Poisoned)));
    assert!(matches!(
        log.append(2, b"must not write", false),
        Err(LogError::Poisoned)
    ));
}

#[test]
fn invalid_position_is_rejected_before_any_engine_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = RaftLog::open(dir.path(), None).unwrap();
    assert!(matches!(
        log.append(2, b"gap", false),
        Err(LogError::InvalidArgument(_))
    ));
    log.append(1, b"first", false).unwrap();
    assert!(matches!(
        log.append(1, b"overwrite", false),
        Err(LogError::InvalidArgument(_))
    ));
    assert_eq!(log.read_range(1, 1, 100).unwrap()[0].payload, b"first");
}

#[test]
fn identical_file_layouts_in_different_stores_never_share_a_cached_entry() {
    // The caller stays on one thread and switches actual engines. Before the
    // scoped-reader workaround, 0.4.2 returned the previous store's block here.
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let epoch = Uuid::new_v4();
    for (path, epoch, payload) in [
        (first.path(), epoch, b"first-data".as_slice()),
        (second.path(), Uuid::new_v4(), b"other-data".as_slice()),
    ] {
        let mut log = RaftLog::open(path, Some(epoch)).unwrap();
        log.append(1, payload, false).unwrap();
        assert_eq!(log.read_range(1, 1, 100).unwrap()[0].payload, payload);
    }
    let mut log = RaftLog::open(first.path(), Some(epoch)).unwrap();
    assert_eq!(log.read_range(1, 1, 100).unwrap()[0].payload, b"first-data");
}

#[test]
fn purge_preserves_live_entries_until_compaction_and_retains_the_accepted_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = LogConfig::desktop(dir.path());
    config.target_file_bytes = 64 * 1024;
    config.purge_threshold_bytes = 128 * 1024;
    config.rewrite_threshold_bytes = 64 * 1024;
    let mut log = RaftLog::open_config(config.clone(), None).unwrap();
    for position in 1..=20 {
        let mut state = position * 0x1234_5678 + 1;
        let payload = (0..48 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect::<Vec<_>>();
        log.append(position, &payload, false).unwrap();
    }
    let before = log.purge().unwrap();
    assert_eq!(
        log.read_range(1, 20, 2 * 1024 * 1024).unwrap().len(),
        20,
        "purge cannot discard live entries just because the threshold is exceeded"
    );
    let suffix = log.read_range(17, 4, 1024 * 1024).unwrap();
    log.compact_prefix(16, true).unwrap();
    let after = log.purge().unwrap();
    println!("rotation/purge probe: before={before:?}, after={after:?}");
    assert_eq!(log.read_range(17, 4, 1024 * 1024).unwrap(), suffix);
    drop(log);
    let mut log = RaftLog::open_config(config, None).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 20);
    assert_eq!(log.head().unwrap().compacted_position, 16);
    assert_eq!(log.read_range(17, 4, 1024 * 1024).unwrap(), suffix);
}

#[test]
fn purge_rewrite_never_reuses_another_stores_identically_positioned_block() {
    for store in 1..=2_u64 {
        let dir = tempfile::tempdir().unwrap();
        let mut config = LogConfig::desktop(dir.path());
        config.target_file_bytes = 64 * 1024;
        config.purge_threshold_bytes = 128 * 1024;
        config.rewrite_threshold_bytes = 64 * 1024;
        let mut log = RaftLog::open_config(config.clone(), None).unwrap();
        for position in 1..=21 {
            let mut random = (position + store * 1024) * 0x1234_5678 + 1;
            let payload = (0..48 * 1024)
                .map(|_| {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    random as u8
                })
                .collect::<Vec<_>>();
            log.append(position, &payload, false).unwrap();
        }
        let tail = log.read_range(20, 2, 1024 * 1024).unwrap();
        log.compact_prefix(19, true).unwrap();
        log.purge().unwrap();
        drop(log);
        let mut log = RaftLog::open_config(config, None).unwrap();
        assert_eq!(log.read_range(20, 2, 1024 * 1024).unwrap(), tail);
    }
}
