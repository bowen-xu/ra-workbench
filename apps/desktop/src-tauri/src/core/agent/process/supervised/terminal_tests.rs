use super::*;
use std::{collections::BTreeMap, future::pending, path::Path};
use tokio::time::{sleep, timeout};
fn spec(root: &Path, script: &str) -> ProcessSpec {
    ProcessSpec {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), script.into()],
        directory: root.into(),
        environment: BTreeMap::new(),
    }
}
fn size() -> TerminalSize {
    TerminalSize {
        rows: 24,
        columns: 80,
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
async fn terminal_uses_whole_budget_and_preserves_head_tail_omission_and_exit() {
    let temp = tempfile::tempdir().unwrap();
    let (process, input) = SupervisedProcess::spawn_terminal(spec(temp.path(),
        "printf HEAD; /bin/dd if=/dev/zero bs=8192 count=20 2>/dev/null; printf TAIL >&2; exit 7"), size(), limits()).unwrap();
    let result = timeout(Duration::from_secs(15), process.finish(pending()))
        .await
        .unwrap();
    drop(input);
    assert_eq!(result.reason, StopReason::Completed);
    assert!(result.drained);
    assert_eq!(result.exit.unwrap().code, Some(7));
    let terminal = result.terminal.unwrap();
    assert_eq!(terminal.head.len(), 512);
    assert_eq!(terminal.tail.len(), 512);
    assert_eq!(&terminal.head[..4], b"HEAD");
    assert_eq!(&terminal.tail[508..], b"TAIL");
    assert_eq!(terminal.omitted_bytes, 8192 * 20 + 8 - 1024);
    for pipe in [result.stdout, result.stderr] {
        assert!(pipe.head.is_empty() && pipe.tail.is_empty());
        assert_eq!(pipe.omitted_bytes, 0);
    }
}
#[tokio::test]
async fn terminal_cancellation_reaps_even_when_input_handle_remains_open() {
    let temp = tempfile::tempdir().unwrap();
    let (process, input) =
        SupervisedProcess::spawn_terminal(spec(temp.path(), "exec /bin/cat"), size(), limits())
            .unwrap();
    let result = timeout(
        Duration::from_secs(5),
        process.finish(sleep(Duration::from_millis(50))),
    )
    .await
    .unwrap();
    drop(input);
    assert_eq!(result.reason, StopReason::Cancelled);
    assert!(result.drained);
    assert_eq!(result.exit.unwrap().signal, Some(nix::libc::SIGKILL));
    assert!(result.terminal.is_some());
}
#[tokio::test]
async fn continuous_terminal_output_cannot_starve_deadline_or_exceed_storage() {
    let temp = tempfile::tempdir().unwrap();
    let mut limits = limits();
    limits.retained_bytes = 1;
    limits.execution_timeout = Duration::from_millis(300);
    let (process, input) = SupervisedProcess::spawn_terminal(
        spec(temp.path(), "while :; do printf data; done"),
        size(),
        limits,
    )
    .unwrap();
    let result = timeout(Duration::from_secs(5), process.finish(pending()))
        .await
        .unwrap();
    drop(input);
    assert_eq!(result.reason, StopReason::Deadline);
    assert!(result.drained);
    let terminal = result.terminal.unwrap();
    assert_eq!(terminal.head.len() + terminal.tail.len(), 1);
    assert!(terminal.omitted_bytes > 0);
}
#[tokio::test]
async fn terminal_input_and_eof_work_while_supervisor_collects_output() {
    let temp = tempfile::tempdir().unwrap();
    let (process, mut input) = SupervisedProcess::spawn_terminal(
        spec(temp.path(), "read line; printf 'received:%s' \"$line\""),
        size(),
        limits(),
    )
    .unwrap();
    let (result, ()) = tokio::join!(process.finish(pending()), async {
        let bytes = b"hello\n";
        let mut offset = 0;
        while offset < bytes.len() {
            offset += input.write(&bytes[offset..]).await.unwrap();
        }
    });
    assert!(result.drained);
    assert_eq!(result.reason, StopReason::Completed);
    let output = result.terminal.unwrap();
    let mut bytes = output.head;
    bytes.extend(output.tail);
    assert!(String::from_utf8_lossy(&bytes).contains("received:hello"));
}
