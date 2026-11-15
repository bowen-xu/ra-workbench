use super::*;
use crate::agent::process::{ProcessLimits, StopReason, SupervisedProcess};
use std::{collections::BTreeMap, fs, future::pending, os::unix::fs::symlink, time::Duration};

fn command(work: &Path, script: &str) -> ProcessSpec {
    ProcessSpec {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), script.into(), "sandbox-test".into()],
        directory: work.into(),
        environment: BTreeMap::new(),
    }
}
fn paths(root: &Path) -> OfflineSandboxPaths {
    OfflineSandboxPaths {
        work: root.join("work"),
        read_only: vec![root.join("read-only")],
        protected: vec![root.join("protected")],
        allow_system_metadata: false,
    }
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for name in ["work", "read-only", "protected", "outside"] {
        fs::create_dir(root.path().join(name)).unwrap();
    }
    fs::write(root.path().join("read-only/input"), "allowed").unwrap();
    fs::write(root.path().join("protected/input"), "fixture-private").unwrap();
    fs::write(root.path().join("outside/input"), "fixture-outside").unwrap();
    root
}
async fn run(command: ProcessSpec) -> crate::agent::process::ProcessOutcome {
    let (process, input) = SupervisedProcess::spawn(
        command,
        ProcessLimits {
            retained_bytes: 8192,
            execution_timeout: Duration::from_secs(10),
            drain_timeout: Duration::from_secs(2),
        },
    )
    .unwrap();
    drop(input);
    let outcome = process.finish(pending()).await;
    assert!(outcome.drained);
    assert_eq!(outcome.reason, StopReason::Completed);
    assert!(outcome.error.is_none());
    outcome
}

#[tokio::test]
async fn permits_work_and_read_grant_but_denies_originals_and_symlink_escape() {
    let fixture = fixture();
    let root = fixture.path().canonicalize().unwrap();
    symlink(root.join("protected/input"), root.join("work/private-link")).unwrap();
    symlink(root.join("outside"), root.join("work/outside-link")).unwrap();
    let mut spec = command(
        &root.join("work"),
        r#"
set -eu
/bin/cat "$1/input" > result
test "$(/bin/cat result)" = allowed
/bin/cp /usr/bin/true ./local-tool
./local-tool
if /bin/cat "$2/input" > /dev/null 2>/dev/null; then exit 21; fi
if /bin/cat "$3/input" > /dev/null 2>/dev/null; then exit 22; fi
if /bin/cat private-link > /dev/null 2>/dev/null; then exit 23; fi
if /bin/sh -c 'echo changed > "$1/input"' sh "$1" 2>/dev/null; then exit 24; fi
if /bin/sh -c 'echo escaped > outside-link/new' 2>/dev/null; then exit 25; fi
printf done
"#,
    );
    spec.arguments.extend([
        root.join("read-only").into_os_string(),
        root.join("protected").into_os_string(),
        root.join("outside").into_os_string(),
    ]);
    let outcome = run(prepare_offline_sandbox(spec, paths(&root)).unwrap()).await;
    assert_eq!(
        outcome.exit.unwrap().code,
        Some(0),
        "stderr: {:?}",
        outcome.stderr
    );
    assert_eq!(fs::read(root.join("work/result")).unwrap(), b"allowed");
    assert_eq!(fs::read(root.join("read-only/input")).unwrap(), b"allowed");
    assert!(!root.join("outside/new").exists());
}

#[tokio::test]
async fn blocks_loopback_network_without_an_unsandboxed_fallback() {
    let fixture = fixture();
    let root = fixture.path().canonicalize().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut spec = command(
        &root.join("work"),
        "exec /usr/bin/curl --noproxy '*' --connect-timeout 1 --max-time 2 \"$1\"",
    );
    spec.arguments
        .push(format!("http://{}/", listener.local_addr().unwrap()).into());
    let outcome = run(prepare_offline_sandbox(spec, paths(&root)).unwrap()).await;
    assert_ne!(outcome.exit.unwrap().code, Some(0));
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    // This asserts a functioning sandboxed curl, not sandbox_apply failure.
    let mut stderr = outcome.stderr.head;
    stderr.extend(outcome.stderr.tail);
    assert!(
        String::from_utf8_lossy(&stderr).contains("curl:"),
        "{stderr:?}"
    );
}

#[test]
fn rejects_overbroad_roots_injected_environment_and_mismatched_cwd() {
    let fixture = fixture();
    let root = fixture.path().canonicalize().unwrap();
    let mut grants = paths(&root);
    grants.read_only.push(root.clone());
    assert!(matches!(
        prepare_offline_sandbox(command(&root.join("work"), "true"), grants),
        Err(ProcessError::Invalid)
    ));
    let mut spec = command(&root.join("work"), "true");
    spec.environment
        .insert("DYLD_INSERT_LIBRARIES".into(), "fixture-only".into());
    assert!(matches!(
        prepare_offline_sandbox(spec, paths(&root)),
        Err(ProcessError::Invalid)
    ));
    assert!(matches!(
        prepare_offline_sandbox(command(&root, "true"), paths(&root)),
        Err(ProcessError::Invalid)
    ));
    symlink(root.join("protected"), root.join("alias")).unwrap();
    let mut grants = paths(&root);
    grants.protected = vec![root.join("alias/not-yet-created")];
    assert!(matches!(
        prepare_offline_sandbox(command(&root.join("work"), "true"), grants),
        Err(ProcessError::Invalid)
    ));
}

#[tokio::test]
async fn sandboxed_changes_are_collected_from_the_owned_workspace_lease() {
    use crate::agent::workspace::WorkspaceStore;
    use research_agent_core::workspace::{FileChangeKind, WorkspaceIdentity, WorkspaceManifest};
    use uuid::Uuid;
    let fixture = fixture();
    let root = fixture.path().canonicalize().unwrap();
    let store = WorkspaceStore::open(&root, |_| Ok(())).unwrap();
    let manifest = WorkspaceManifest {
        schema_version: 1,
        files: vec![],
        identity: WorkspaceIdentity {
            lease_id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            environment_id: "sandbox-fixture".into(),
        },
    };
    let lease = store.create(manifest.clone(), vec![]).unwrap();
    let work = lease.working_directory().unwrap();
    let base = work.parent().unwrap().join("base");
    let outcome = run(prepare_offline_sandbox(
        command(&work, "printf 'candidate' > candidate.tex"),
        OfflineSandboxPaths {
            work,
            read_only: vec![],
            protected: vec![base, root.join("protected")],
            allow_system_metadata: false,
        },
    )
    .unwrap())
    .await;
    assert_eq!(outcome.exit.unwrap().code, Some(0));
    let changes = lease.changes().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "candidate.tex");
    assert_eq!(changes[0].kind, FileChangeKind::Added);
    drop(lease);
    // Parallel process tests can briefly inherit this CLOEXEC lock between
    // fork and exec. Busy is valid during that interval; it must clear once
    // those trusted launch hooks finish, not be treated as data corruption.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match store.discard(&manifest.identity) {
                Ok(()) => break,
                Err(crate::agent::workspace::LeaseError::Busy) => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(error) => panic!("workspace cleanup failed: {error}"),
            }
        }
    })
    .await
    .unwrap();
}
