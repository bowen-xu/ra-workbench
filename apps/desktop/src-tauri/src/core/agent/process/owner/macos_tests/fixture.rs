use super::super::*;
use std::{os::unix::process::CommandExt, process::Stdio};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::ChildStdin,
    time::{Duration, timeout},
};

pub(super) const WAIT: Duration = Duration::from_secs(8);

pub(super) struct OwnedChild {
    pub child: Child,
    pub input: Option<ChildStdin>,
    pub pid: Pid,
}

pub(super) async fn spawn(group: i32) -> OwnedChild {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "printf 'ready\\n'; IFS= read -r ignored"])
        .env_clear()
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command.as_std_mut().process_group(group);
    let mut child = command.spawn().unwrap();
    let pid = Pid::from_raw(i32::try_from(child.id().unwrap()).unwrap());
    let actual = nix::unistd::getpgid(Some(pid)).unwrap();
    assert_eq!(
        actual.as_raw(),
        if group == 0 { pid.as_raw() } else { group }
    );
    assert_ne!(actual, nix::unistd::getpgrp());
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    timeout(WAIT, output.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line, "ready\n");
    OwnedChild {
        input: child.stdin.take(),
        child,
        pid,
    }
}

pub(super) async fn observe_exit_without_reap(pid: Pid) {
    // The caller retains the Child and never polls wait/try_wait or drops it
    // during this observation. Tokio 1.53.1's Darwin SignalReaper only reaps
    // when polled or dropped; reading the separate stdout handle does neither.
    // Failure unwinding retains kill_on_drop and ProcessOwner's group Drop;
    // no additional task drives this Child's wait operation.
    timeout(
        WAIT,
        tokio::task::spawn_blocking(move || {
            // SAFETY: zero is valid initial siginfo_t storage, kept writable
            // through waitid. pid identifies this test's owned direct child.
            let mut info: nix::libc::siginfo_t = unsafe { std::mem::zeroed() };
            // SAFETY: no other waiter runs for this child. WNOWAIT observes
            // its exit without stealing the production owner's reap operation.
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
            // SAFETY: successful waitid(WEXITED) initialized these SIGCHLD fields.
            let (actual, status) = unsafe { (info.si_pid(), info.si_status()) };
            assert_eq!(actual, pid.as_raw());
            assert_eq!(status, 1);
        }),
    )
    .await
    .unwrap()
    .unwrap();
}

pub(super) fn natural_exit() -> ProcessExit {
    ProcessExit {
        code: Some(1),
        signal: None,
    }
}

pub(super) fn assert_denied(error: &ProcessError, expected: ProcessIoOperation) {
    assert!(matches!(
        error,
        ProcessError::OperationIo {
            operation,
            kind: io::ErrorKind::PermissionDenied,
            raw_os_error: Some(nix::libc::EPERM),
        } if *operation == expected
    ));
}
