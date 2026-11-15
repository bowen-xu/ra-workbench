use super::*;
use crate::agent::process::process_control;
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
fn limits() -> ProcessLimits {
    ProcessLimits {
        retained_bytes: 1024,
        execution_timeout: Duration::from_secs(5),
        drain_timeout: Duration::from_secs(2),
    }
}
#[tokio::test]
async fn snapshots_do_not_consume_output_and_closed_owner_rejects_requests() {
    let temp = tempfile::tempdir().unwrap();
    let (process, mut input) = SupervisedProcess::spawn(
        spec(temp.path(), "printf READY; read line; printf DONE"),
        limits(),
    )
    .unwrap();
    let (mut control, commands) = process_control();
    let (outcome, ()) = timeout(Duration::from_secs(8), async {
        tokio::join!(process.finish_controlled(pending(), commands), async {
            loop {
                let snapshot = control.snapshot().await.unwrap();
                assert!(snapshot.terminal.is_none());
                if snapshot.stdout.head == b"READY" {
                    break;
                }
                sleep(Duration::from_millis(5)).await;
            }
            assert_eq!(control.snapshot().await.unwrap().stdout.head, b"READY");
            assert!(matches!(
                control
                    .resize(TerminalSize {
                        rows: 24,
                        columns: 80
                    })
                    .await,
                Err(ProcessError::Unsupported)
            ));
            input.write(b"\n").await.unwrap();
            input.close();
        })
    })
    .await
    .unwrap();
    assert!(outcome.drained);
    assert_eq!(outcome.stdout.head, b"READYDONE");
    assert!(matches!(
        control.snapshot().await,
        Err(ProcessError::Io(std::io::ErrorKind::BrokenPipe))
    ));
}
#[tokio::test]
async fn controlled_terminal_resize_is_visible_to_child() {
    let temp = tempfile::tempdir().unwrap();
    let (process, mut input) = SupervisedProcess::spawn_terminal(
        spec(temp.path(), "read line; /bin/stty size"),
        TerminalSize {
            rows: 24,
            columns: 80,
        },
        limits(),
    )
    .unwrap();
    let (mut control, commands) = process_control();
    let (outcome, ()) = tokio::join!(process.finish_controlled(pending(), commands), async {
        assert!(
            control
                .resize(TerminalSize {
                    rows: 0,
                    columns: 80
                })
                .await
                .is_err()
        );
        control
            .resize(TerminalSize {
                rows: 40,
                columns: 120,
            })
            .await
            .unwrap();
        input.write(b"\n").await.unwrap();
    });
    assert!(outcome.drained);
    let output = outcome.terminal.unwrap();
    assert!(String::from_utf8_lossy(&output.head).contains("40 120"));
}
#[tokio::test]
async fn control_interrupt_reaps_and_dropping_client_does_not_stop_draining() {
    let temp = tempfile::tempdir().unwrap();
    let (process, input) =
        SupervisedProcess::spawn(spec(temp.path(), "exec /bin/sleep 30"), limits()).unwrap();
    let (mut control, commands) = process_control();
    let (outcome, ()) = tokio::join!(process.finish_controlled(pending(), commands), async {
        control.interrupt().await.unwrap();
        drop(control);
    });
    drop(input);
    assert!(outcome.drained);
    assert_eq!(outcome.exit.unwrap().signal, Some(nix::libc::SIGINT));
}
#[tokio::test]
async fn snapshot_pressure_cannot_extend_execution_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let mut budget = limits();
    budget.execution_timeout = Duration::from_millis(300);
    budget.retained_bytes = 16;
    let (process, input) =
        SupervisedProcess::spawn(spec(temp.path(), "while :; do printf DATA; done"), budget)
            .unwrap();
    let (mut control, commands) = process_control();
    let (outcome, count) = timeout(Duration::from_secs(4), async {
        tokio::join!(process.finish_controlled(pending(), commands), async {
            let mut count = 0;
            while let Ok(snapshot) = control.snapshot().await {
                assert!(snapshot.stdout.head.len() + snapshot.stdout.tail.len() <= 8);
                count += 1;
            }
            count
        })
    })
    .await
    .unwrap();
    drop(input);
    assert!(count > 0);
    assert_eq!(outcome.reason, StopReason::Deadline);
    assert!(outcome.drained);
    assert!(outcome.stdout.omitted_bytes > 0);
}
