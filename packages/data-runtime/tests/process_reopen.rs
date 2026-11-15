//! A separate test binary does not fork the parallel unit-test process while
//! unrelated temporary engines hold directory locks. The crash probe still
//! bypasses all Engine/Rust destructors and runs the actual native filesystem.
use research_data_runtime::RaftLog;

#[test]
fn process_exit_without_engine_drop_recovers_every_acknowledged_small_write() {
    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "process_exit_writer", "--nocapture"])
        .env(
            "RESEARCH_DATA_RUNTIME_EXIT_TEST_PATH",
            dir.path().join("log"),
        )
        .status()
        .unwrap();
    assert!(status.success());
    let mut log = RaftLog::open(dir.path().join("log"), None).unwrap();
    assert_eq!(log.head().unwrap().accepted_position, 17);
    assert_eq!(log.read_range(1, 17, 4096).unwrap().len(), 17);
}

#[test]
fn process_exit_writer() {
    let Some(path) = std::env::var_os("RESEARCH_DATA_RUNTIME_EXIT_TEST_PATH") else {
        return;
    };
    let mut log = RaftLog::open(path, None).unwrap();
    for position in 1..=17 {
        log.append(position, format!("mutation-{position}").as_bytes(), false)
            .unwrap();
    }
    // Process recovery, not graceful close or stable-device durability.
    std::process::exit(0);
}
