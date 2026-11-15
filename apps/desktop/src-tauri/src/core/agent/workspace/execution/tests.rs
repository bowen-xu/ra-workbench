use super::*;
use crate::agent::workspace::WorkspaceStore;
use research_agent_core::workspace::{WorkspaceIdentity, WorkspaceManifest};
use std::{collections::BTreeMap, time::Duration};
use tokio::time::{sleep, timeout};
use uuid::Uuid;
fn setup(root: &Path) -> super::super::WorkspaceLease {
    WorkspaceStore::open(&root.canonicalize().unwrap(), |_| Ok(()))
        .unwrap()
        .create(
            WorkspaceManifest {
                schema_version: 1,
                identity: WorkspaceIdentity {
                    lease_id: Uuid::new_v4(),
                    conversation_id: Uuid::new_v4(),
                    turn_id: Uuid::new_v4(),
                    environment_id: "test".into(),
                },
                files: vec![],
            },
            vec![],
        )
        .unwrap()
}
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
        execution_timeout: Duration::from_secs(5),
        drain_timeout: Duration::from_secs(2),
    }
}
#[tokio::test]
async fn hook_pipe_cancellation_reaps_before_releasing_writer_intent() {
    let temp = tempfile::tempdir().unwrap();
    let mut lease = setup(temp.path());
    let guard = lease.begin_execution().unwrap();
    let command = spec(guard.working_directory(), "exec /bin/sleep 30");
    let (process, mut input) = guard.spawn_hook(command, limits()).unwrap();
    input.close();
    assert_eq!(lease.changes(), Err(LeaseError::Busy));
    let outcome = process.finish(async {}).await;
    assert_eq!(outcome.reason, crate::agent::process::StopReason::Cancelled);
    assert!(outcome.drained && outcome.exit.is_some() && outcome.resource_error.is_none());
    assert!(lease.changes().unwrap().is_empty());
}

#[tokio::test]
async fn dropped_hook_supervisor_preserves_intent_without_claiming_quiescence() {
    let temp = tempfile::tempdir().unwrap();
    let mut lease = setup(temp.path());
    let guard = lease.begin_execution().unwrap();
    let command = spec(guard.working_directory(), "exec /bin/sleep 30");
    let (process, input) = guard.spawn_hook(command, limits()).unwrap();
    drop(input);
    drop(process);
    assert_eq!(lease.changes(), Err(LeaseError::Busy));
}
#[tokio::test]
async fn drop_cancels_and_clears_intent_only_after_supervised_reaping() {
    let temp = tempfile::tempdir().unwrap();
    let mut lease = setup(temp.path());
    let guard = lease.begin_execution().unwrap();
    let command = spec(guard.working_directory(), "exec /bin/sleep 30");
    let session = guard.spawn(command, limits(), None).unwrap();
    assert_eq!(lease.changes(), Err(LeaseError::Busy));
    drop(session);
    timeout(Duration::from_secs(4), async {
        loop {
            match lease.changes() {
                Err(LeaseError::Busy) => sleep(Duration::from_millis(5)).await,
                result => {
                    assert!(result.unwrap().is_empty());
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn changed_marker_is_preserved_and_release_error_does_not_hide_exit() {
    let temp = tempfile::tempdir().unwrap();
    let mut lease = setup(temp.path());
    let guard = lease.begin_execution().unwrap();
    let path = guard.working_directory().to_owned();
    let mut session = guard
        .spawn(spec(&path, "read line; exit 7"), limits(), None)
        .unwrap();
    let marker = path.parent().unwrap().join(MARKER);
    std::fs::rename(&marker, marker.with_extension("old")).unwrap();
    std::fs::write(&marker, "replacement").unwrap();
    session.write(b"\n").await.unwrap();
    let result = session.finish().await.unwrap();
    assert!(result.drained);
    assert_eq!(result.exit.unwrap().code, Some(7));
    assert!(result.error.is_none());
    assert!(matches!(
        result.resource_error,
        Some(ProcessError::ResourceRelease)
    ));
    assert_eq!(std::fs::read(&marker).unwrap(), b"replacement");
    assert_eq!(lease.changes(), Err(LeaseError::Busy));
}
#[tokio::test]
async fn mismatched_cwd_never_launches_and_leaves_intent_for_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut lease = setup(temp.path());
    let guard = lease.begin_execution().unwrap();
    assert!(matches!(
        guard.spawn(spec(outside.path(), "touch launched"), limits(), None),
        Err(ProcessError::Invalid)
    ));
    assert!(!outside.path().join("launched").exists());
    assert_eq!(lease.changes(), Err(LeaseError::Busy));
}
#[tokio::test]
async fn bound_terminal_releases_intent_and_preserves_terminal_output() {
    let temp = tempfile::tempdir().unwrap();
    let mut lease = setup(temp.path());
    let guard = lease.begin_execution().unwrap();
    let command = spec(guard.working_directory(), "/bin/stty size");
    let mut session = guard
        .spawn(
            command,
            limits(),
            Some(TerminalSize {
                rows: 40,
                columns: 120,
            }),
        )
        .unwrap();
    let result = session.finish().await.unwrap();
    assert!(result.drained && result.resource_error.is_none());
    assert!(String::from_utf8_lossy(&result.terminal.as_ref().unwrap().head).contains("40 120"));
    assert!(lease.changes().unwrap().is_empty());
}
