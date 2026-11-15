use super::*;
use std::{collections::BTreeMap, future::pending, path::Path};
use tokio::time::{sleep, timeout};

fn spec(path: &Path, script: &str) -> ProcessSpec {
    ProcessSpec {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), script.into()],
        directory: path.into(),
        environment: BTreeMap::new(),
    }
}
fn limits() -> ProcessLimits {
    ProcessLimits {
        retained_bytes: 1024,
        execution_timeout: Duration::from_secs(10),
        drain_timeout: Duration::from_secs(2),
    }
}
#[tokio::test]
async fn drains_large_binary_streams_with_bounded_storage_and_retains_exit() {
    let temp = tempfile::tempdir().unwrap();
    let (process, input) = SupervisedProcess::spawn(spec(temp.path(),
        "/bin/dd if=/dev/zero bs=8192 count=20 2>/dev/null; /bin/dd if=/dev/zero bs=8192 count=20 >&2 2>/dev/null; exit 7"), limits()).unwrap();
    drop(input);
    let outcome = timeout(Duration::from_secs(15), process.finish(pending()))
        .await
        .unwrap();
    assert_eq!(outcome.reason, StopReason::Completed);
    assert!(outcome.terminal.is_none());
    assert!(outcome.drained);
    assert_eq!(outcome.exit.unwrap().code, Some(7));
    assert!(outcome.error.is_none());
    for output in [outcome.stdout, outcome.stderr] {
        assert_eq!(output.head, vec![0; 256]);
        assert_eq!(output.tail, vec![0; 256]);
        assert_eq!(output.omitted_bytes, 8192 * 20 - 512);
    }
}
#[tokio::test]
async fn cancellation_kills_and_reaps_instead_of_claiming_signal_is_completion() {
    let temp = tempfile::tempdir().unwrap();
    let (process, input) =
        SupervisedProcess::spawn(spec(temp.path(), "exec /bin/cat"), limits()).unwrap();
    let outcome = timeout(
        Duration::from_secs(5),
        process.finish(sleep(Duration::from_millis(30))),
    )
    .await
    .unwrap();
    drop(input);
    assert_eq!(outcome.reason, StopReason::Cancelled);
    assert!(outcome.drained);
    assert_eq!(outcome.exit.unwrap().signal, Some(nix::libc::SIGKILL));
    assert!(outcome.error.is_none());
}
#[tokio::test]
async fn deadline_is_not_starved_by_a_continuous_output_stream() {
    let temp = tempfile::tempdir().unwrap();
    let mut limits = limits();
    limits.execution_timeout = Duration::from_millis(100);
    limits.retained_bytes = 0;
    let (mut process, input) = SupervisedProcess::spawn(
        spec(temp.path(), "while :; do printf 'output'; done"),
        limits,
    )
    .unwrap();
    drop(input);
    // Start this starvation test only once the producer is demonstrably active.
    // Process startup can itself exceed 100 ms under concurrent native tests;
    // production still includes startup in its execution deadline.
    let first = timeout(Duration::from_secs(5), process.process.next_output())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let TransportOutput::Pipe(first) = first else {
        panic!("expected pipe output")
    };
    assert!(!first.bytes.is_empty());
    process.stdout.push(&first.bytes);
    process.deadline = Instant::now() + Duration::from_millis(100);
    let outcome = timeout(Duration::from_secs(5), process.finish(pending()))
        .await
        .unwrap();
    assert_eq!(outcome.reason, StopReason::Deadline);
    assert!(outcome.drained);
    assert!(outcome.stdout.head.is_empty() && outcome.stdout.tail.is_empty());
    assert!(outcome.stdout.omitted_bytes > 0);
    assert!(outcome.exit.is_some());
}
#[tokio::test]
async fn stdin_can_progress_while_supervisor_drains() {
    let temp = tempfile::tempdir().unwrap();
    let (process, mut input) =
        SupervisedProcess::spawn(spec(temp.path(), "exec /bin/cat"), limits()).unwrap();
    let (outcome, ()) = tokio::join!(process.finish(pending()), async {
        let bytes = vec![b'x'; 256 * 1024];
        let mut offset = 0;
        while offset < bytes.len() {
            offset += input.write(&bytes[offset..]).await.unwrap();
        }
        input.close();
    });
    assert!(outcome.drained);
    assert_eq!(outcome.reason, StopReason::Completed);
    assert_eq!(outcome.stdout.omitted_bytes, 256 * 1024 - 512);
}
#[test]
fn invalid_limits_reject_before_spawn() {
    let temp = tempfile::tempdir().unwrap();
    let mut invalid = limits();
    invalid.retained_bytes = 16 * 1024 * 1024 + 1;
    assert!(matches!(
        SupervisedProcess::spawn(spec(temp.path(), "touch unexpected"), invalid),
        Err(ProcessError::Invalid)
    ));
    assert!(!temp.path().join("unexpected").exists());
}

#[tokio::test]
async fn exhausted_drain_deadline_does_not_report_complete_reaping() {
    let temp = tempfile::tempdir().unwrap();
    let mut limits = limits();
    limits.drain_timeout = Duration::from_nanos(1);
    let (process, input) =
        SupervisedProcess::spawn(spec(temp.path(), "exec /bin/cat"), limits).unwrap();
    let outcome = process.finish(std::future::ready(())).await;
    drop(input);
    assert_eq!(outcome.reason, StopReason::Cancelled);
    assert!(!outcome.drained);
    assert!(outcome.exit.is_none());
}
