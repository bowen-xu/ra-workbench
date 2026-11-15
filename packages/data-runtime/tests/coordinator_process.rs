#[path = "coordinator_fixture.rs"]
mod fixture;
use fixture::*;
use research_data_runtime::coordinator::WriteOptions;
use std::sync::atomic::Ordering;
use uuid::Uuid;

#[test]
fn child_accept_exit() {
    let Some(directory) = std::env::var_os("RESEARCH_COORDINATOR_CRASH_DIRECTORY") else {
        return;
    };
    let epoch =
        Uuid::parse_str(&std::env::var("RESEARCH_COORDINATOR_CRASH_EPOCH").unwrap()).unwrap();
    let fake = projector(epoch);
    let runtime = open(std::path::Path::new(&directory), &fake, config());
    runtime
        .write(command(0, 9), WriteOptions::default())
        .unwrap();
    assert_eq!(fake.sql.lock().unwrap().position, 0);
    std::process::exit(0); // No Rust drop/close/flush of the coordinator or engine.
}

#[test]
fn accepted_automatic_write_survives_process_exit_without_shutdown_or_sql_apply() {
    let directory = tempfile::tempdir().unwrap();
    let epoch = Uuid::new_v4();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_accept_exit", "--nocapture"])
        .env("RESEARCH_COORDINATOR_CRASH_DIRECTORY", directory.path())
        .env("RESEARCH_COORDINATOR_CRASH_EPOCH", epoch.to_string())
        .status()
        .unwrap();
    assert!(status.success());
    let fake = projector(epoch);
    let _restored = open(directory.path(), &fake, config());
    assert_eq!(
        (
            fake.sql.lock().unwrap().position,
            fake.faults.allocations.load(Ordering::SeqCst)
        ),
        (1, 0)
    );
    assert_eq!(fake.sql.lock().unwrap().total, 9);
}
