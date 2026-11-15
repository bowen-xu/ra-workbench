use super::*;

#[test]
fn only_preparing_may_initialize_metadata_after_the_empty_engine_was_created() {
    let directory = tempfile::tempdir().unwrap();
    let config = LogConfig::desktop(directory.path());
    // The real published engine writes its own empty queue headers. Simulate a
    // process stop before the adapter's first runtime-metadata batch exists.
    let engine = Engine::open(config.engine_config().unwrap()).unwrap();
    assert!(engine.is_empty());
    drop(engine);
    let epoch = Uuid::new_v4();
    assert!(matches!(
        RaftLog::open(directory.path(), Some(epoch)),
        Err(LogError::Corrupt(_))
    ));
    let io = LogIoCounters::default();
    let log = RaftLog::open_preparing_with_io(directory.path(), epoch, io.clone()).unwrap();
    assert!(io.snapshot().write_bytes > 0 && io.snapshot().sync_calls > 0);
    assert_eq!(log.head().unwrap().epoch, epoch);
    assert_eq!(log.head().unwrap().accepted_position, 0);
    drop(log);
    assert_eq!(
        RaftLog::open(directory.path(), Some(epoch))
            .unwrap()
            .head()
            .unwrap()
            .epoch,
        epoch
    );
}

#[test]
fn preparing_refuses_nonempty_engine_without_metadata_and_preserves_its_data() {
    let directory = tempfile::tempdir().unwrap();
    let config = LogConfig::desktop(directory.path());
    let engine = Engine::open(config.engine_config().unwrap()).unwrap();
    let mut batch = LogBatch::default();
    batch
        .put(GROUP, b"foreign-key".to_vec(), b"foreign-data".to_vec())
        .unwrap();
    engine.write(&mut batch, true).unwrap();
    drop(engine);
    assert!(matches!(
        RaftLog::open_preparing(directory.path(), Uuid::new_v4()),
        Err(LogError::Corrupt(_))
    ));
    let engine = Engine::open(config.engine_config().unwrap()).unwrap();
    assert_eq!(engine.get(GROUP, b"foreign-key").unwrap(), b"foreign-data");
    assert!(engine.get(GROUP, STATE_KEY).is_none());
}

#[test]
fn preparing_refuses_an_already_accepted_business_prefix_and_a_nil_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let epoch = Uuid::new_v4();
    let mut log = RaftLog::open(directory.path(), Some(epoch)).unwrap();
    log.append(1, b"accepted-record", false).unwrap();
    drop(log);
    assert!(matches!(
        RaftLog::open_preparing(directory.path(), epoch),
        Err(LogError::Corrupt(_))
    ));
    let mut log = RaftLog::open(directory.path(), Some(epoch)).unwrap();
    assert_eq!(
        log.read_range(1, 1, 100).unwrap()[0].payload,
        b"accepted-record"
    );
    let missing = directory.path().join("never-created");
    assert!(matches!(
        RaftLog::open_preparing(&missing, Uuid::nil()),
        Err(LogError::InvalidArgument(_))
    ));
    assert!(!missing.exists());
}
