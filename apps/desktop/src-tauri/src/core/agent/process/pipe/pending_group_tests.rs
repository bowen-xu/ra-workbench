//! An owned root's cached exit must not hide actual pending group cleanup.
use super::*;
use crate::agent::process::ProcessIoOperation;
use std::{io, os::unix::process::CommandExt};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, ChildStdin, Command},
    time::{Duration, timeout},
};

const WAIT: Duration = Duration::from_secs(8);

struct OwnedChild {
    child: Child,
    input: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr: ChildStderr,
    pid: Pid,
}

async fn spawn(group: i32) -> OwnedChild {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "printf 'ready\\n'; IFS= read -r ignored"])
        .env_clear()
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.as_std_mut().process_group(group);
    let mut child = command.spawn().unwrap();
    let pid = Pid::from_raw(i32::try_from(child.id().unwrap()).unwrap());
    let actual_group = nix::unistd::getpgid(Some(pid)).unwrap();
    assert_eq!(
        actual_group.as_raw(),
        if group == 0 { pid.as_raw() } else { group }
    );
    assert_ne!(actual_group, nix::unistd::getpgrp());
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(WAIT, output.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line, "ready\n");
    assert!(output.buffer().is_empty());
    OwnedChild {
        input: child.stdin.take(),
        stderr: child.stderr.take().unwrap(),
        stdout: output.into_inner(),
        child,
        pid,
    }
}

async fn observe_exit_without_reap(pid: Pid) {
    // The caller retains Child without polling wait/try_wait. The Darwin
    // Tokio SignalReaper is not driven by reads from its separate stdout.
    timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            // SAFETY: valid writable siginfo storage lives through waitid;
            // pid came from this test's still-owned direct child.
            let mut info: nix::libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: no other waiter drives this Child. WNOWAIT observes
            // its exit without reaping before the production Pipe owner.
            let result = unsafe {
                nix::libc::waitid(
                    nix::libc::P_PID,
                    u32::try_from(pid.as_raw()).unwrap(),
                    &mut info,
                    nix::libc::WEXITED | nix::libc::WNOWAIT,
                )
            };
            assert_eq!(result, 0, "{}", io::Error::last_os_error());
            assert_eq!(info.si_code, nix::libc::CLD_EXITED);
            // SAFETY: successful WEXITED initialized the SIGCHLD fields.
            let (actual, status) = unsafe { (info.si_pid(), info.si_status()) };
            assert_eq!(actual, pid.as_raw());
            assert_eq!(status, 1);
        }),
    )
    .await
    .unwrap()
    .unwrap();
}

fn assert_pending_cleanup_error(error: ProcessError) {
    assert!(matches!(
        error,
        ProcessError::OperationIo {
            operation: ProcessIoOperation::GroupCleanupAfterWait,
            kind: io::ErrorKind::PermissionDenied,
            raw_os_error: Some(nix::libc::EPERM),
        }
    ));
}

#[tokio::test]
async fn next_output_keeps_pending_group_failure_after_caching_the_root_exit() {
    let root = spawn(0).await;
    let mut sibling = spawn(root.pid.as_raw()).await;
    // Use actual production fields and real remaining stdout/stderr handles.
    // No exit/group state is assigned by the fixture.
    let mut process = PipeProcess {
        owner: ProcessOwner::new(root.child).unwrap(),
        stdout: root.stdout,
        stderr: root.stderr,
        stdout_done: false,
        stderr_done: false,
    };
    drop(root.input);
    observe_exit_without_reap(root.pid).await;
    drop(sibling.input.take());
    observe_exit_without_reap(sibling.pid).await;

    let first = timeout(WAIT, process.next_output())
        .await
        .unwrap()
        .unwrap_err();
    assert_pending_cleanup_error(first);
    assert_eq!(process.exit().unwrap().code, Some(1));
    assert!(!process.owner.is_settled());

    let second = timeout(WAIT, process.next_output())
        .await
        .unwrap()
        .unwrap_err();
    assert_pending_cleanup_error(second);
    assert!(!process.owner.is_settled());

    // Only actual sibling reap changes the observed group state. This is an
    // explicit recovery step rather than a timed or random test retry.
    assert_eq!(sibling.child.try_wait().unwrap().unwrap().code(), Some(1));
    assert!(
        timeout(WAIT, process.next_output())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert!(process.owner.is_settled());
    assert!(process.next_output().await.unwrap().is_none());
}
