use super::*;
use crate::agent::process::StopReason;
use std::{collections::BTreeMap, path::Path};
use tokio::time::sleep;
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
        execution_timeout: Duration::from_secs(10),
        drain_timeout: Duration::from_secs(2),
    }
}
#[tokio::test]
async fn unattended_session_drains_and_yields_without_consuming_final_output() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = ProcessSession::spawn(spec(temp.path(), "/bin/dd if=/dev/zero bs=8192 count=32 2>/dev/null; printf yes > ready; read line; printf END; exit 7"), limits()).unwrap();
    timeout(Duration::from_secs(4), async {
        while !temp.path().join("ready").exists() {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let ProcessPoll::Running(snapshot) = session.poll(Duration::ZERO).await.unwrap() else {
        panic!("expected waiting for input");
    };
    assert!(snapshot.stdout.omitted_bytes > 0);
    assert!(snapshot.terminal.is_none());
    assert!(session.poll(Duration::from_secs(61)).await.is_err());
    session.write(b"\n").await.unwrap();
    session.close_input();
    let result = session.finish().await.unwrap();
    assert_eq!(result.reason, StopReason::Completed);
    assert!(result.drained);
    assert_eq!(result.exit.unwrap().code, Some(7));
    assert!(result.stdout.tail.ends_with(b"END"));
    assert_eq!(result.stdout.omitted_bytes, 8192 * 32 + 3 - 512);
    let again = session.finish().await.unwrap();
    assert!(Arc::ptr_eq(&result, &again));
    assert!(session.write(b"late").await.is_err());
    assert!(matches!(
        session.poll(Duration::ZERO).await.unwrap(),
        ProcessPoll::Finished(_)
    ));
}
#[tokio::test]
async fn cancelled_observation_keeps_session_owned_and_shutdown_is_repeatable() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = ProcessSession::spawn(spec(temp.path(), "exec /bin/cat"), limits()).unwrap();
    assert!(
        timeout(
            Duration::from_millis(20),
            session.poll(Duration::from_secs(1))
        )
        .await
        .is_err()
    );
    session.write(b"alive\n").await.unwrap();
    let result = session.shutdown().await.unwrap();
    assert_eq!(result.reason, StopReason::Cancelled);
    assert!(result.drained);
    assert!(Arc::ptr_eq(&result, &session.shutdown().await.unwrap()));
}
#[tokio::test]
async fn terminal_session_keeps_input_resize_and_output_semantics() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = ProcessSession::spawn_terminal(
        spec(
            temp.path(),
            "read line; /bin/stty size; printf '%s' \"$line\"",
        ),
        TerminalSize {
            rows: 24,
            columns: 80,
        },
        limits(),
    )
    .unwrap();
    session
        .resize(TerminalSize {
            rows: 40,
            columns: 120,
        })
        .await
        .unwrap();
    session.write(b"hello\n").await.unwrap();
    let result = session.finish().await.unwrap();
    assert!(result.drained);
    let terminal = result.terminal.as_ref().unwrap();
    let text = String::from_utf8_lossy(&terminal.head);
    assert!(text.contains("40 120"));
    assert!(text.contains("hello"));
    assert!(result.stdout.head.is_empty());
}
#[tokio::test]
async fn dropping_session_requests_cleanup_without_aborting_reaping() {
    let temp = tempfile::tempdir().unwrap();
    let session = ProcessSession::spawn(
        spec(temp.path(), "echo $$ > pid; exec /bin/sleep 30"),
        limits(),
    )
    .unwrap();
    let pid = timeout(Duration::from_secs(4), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(temp.path().join("pid"))
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                break pid;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    drop(session);
    timeout(Duration::from_secs(4), async {
        loop {
            match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
                Err(nix::errno::Errno::ESRCH) => break,
                Ok(()) => sleep(Duration::from_millis(5)).await,
                other => panic!("unexpected process probe: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
}
