use research_data_runtime::{LogConfig, LogIoCounters, LogIoSnapshot, RaftLog};

#[test]
fn observers_cover_initialization_and_close_without_mixing_independent_log_owners() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first = LogIoCounters::default();
    let second = LogIoCounters::default();
    assert_eq!(first.snapshot(), LogIoSnapshot::default());
    let mut a =
        RaftLog::open_config_with_io(LogConfig::desktop(first_dir.path()), None, first.clone())
            .unwrap();
    let b =
        RaftLog::open_config_with_io(LogConfig::desktop(second_dir.path()), None, second.clone())
            .unwrap();
    let initialized = first.snapshot();
    assert!(
        initialized.write_calls > 0 && initialized.write_bytes > 0 && initialized.sync_calls > 0
    );
    assert_eq!(a.io_counters().snapshot(), initialized);
    let untouched = second.snapshot();
    a.append(1, b"process accepted", false).unwrap();
    assert!(first.snapshot().write_bytes > initialized.write_bytes);
    assert_eq!(first.snapshot().sync_calls, initialized.sync_calls);
    a.sync().unwrap();
    let before_close = first.snapshot();
    assert!(before_close.sync_calls > initialized.sync_calls);
    let epoch = a.head().unwrap().epoch;
    drop(a);
    let closed = first.snapshot();
    assert!(
        closed.write_calls >= before_close.write_calls
            && closed.sync_calls >= before_close.sync_calls
    );
    assert_eq!(second.snapshot(), untouched);
    drop(b);
    // Recovery receives a new observer before open, never a directory-global
    // registry. Its reads and later close cannot mutate the previous handle.
    let recovery = LogIoCounters::default();
    let mut reopened = RaftLog::open_config_with_io(
        LogConfig::desktop(first_dir.path()),
        Some(epoch),
        recovery.clone(),
    )
    .unwrap();
    assert_eq!(
        reopened.read_range(1, 1, 1024).unwrap()[0].payload,
        b"process accepted"
    );
    reopened.append(2, b"recovered owner", true).unwrap();
    assert!(recovery.snapshot().write_bytes > 0 && recovery.snapshot().sync_calls > 0);
    drop(reopened);
    assert_eq!(first.snapshot(), closed);
}
