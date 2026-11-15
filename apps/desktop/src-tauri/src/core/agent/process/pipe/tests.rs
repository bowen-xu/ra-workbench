use super::*;
use tokio::time::{Duration, timeout};

fn spec(directory: &std::path::Path, script: &str) -> ProcessSpec {
    ProcessSpec {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), script.into(), "agent-process-test".into()],
        directory: directory.into(),
        environment: BTreeMap::new(),
    }
}
async fn drain(process: &mut PipeProcess) -> (Vec<u8>, Vec<u8>) {
    timeout(Duration::from_secs(15), async {
        let mut stdout = vec![];
        let mut stderr = vec![];
        while let Some(chunk) = process.next_output().await.unwrap() {
            assert!(chunk.bytes.len() <= 8192);
            match chunk.stream {
                OutputStream::Stdout => stdout.extend(chunk.bytes),
                OutputStream::Stderr => stderr.extend(chunk.bytes),
            }
        }
        (stdout, stderr)
    })
    .await
    .expect("process did not finish and drain")
}
#[tokio::test]
async fn captures_both_byte_streams_and_exit_after_the_complete_tail() {
    let temp = tempfile::tempdir().unwrap();
    let (mut process, stdin) = PipeProcess::spawn(spec(temp.path(),
        "/bin/dd if=/dev/zero bs=8192 count=20 2>/dev/null; /bin/dd if=/dev/zero bs=8192 count=20 >&2 2>/dev/null; exit 7")).unwrap();
    drop(stdin);
    let (stdout, stderr) = drain(&mut process).await;
    assert_eq!(stdout, vec![0; 8192 * 20]);
    assert_eq!(stderr, vec![0; 8192 * 20]);
    assert_eq!(
        process.exit(),
        Some(ProcessExit {
            code: Some(7),
            signal: None
        })
    );
    assert!(process.next_output().await.unwrap().is_none());
}
#[tokio::test]
async fn stdin_and_output_can_progress_together_without_dropped_read_futures() {
    let temp = tempfile::tempdir().unwrap();
    let (mut process, mut stdin) = PipeProcess::spawn(spec(temp.path(), "exec /bin/cat")).unwrap();
    assert!(
        timeout(Duration::from_millis(25), process.next_output())
            .await
            .is_err()
    );
    assert!(process.exit().is_none());
    let bytes = vec![b'x'; 256 * 1024];
    let (output, ()) = tokio::join!(drain(&mut process), async {
        let mut offset = 0;
        while offset < bytes.len() {
            let count = stdin.write(&bytes[offset..]).await.unwrap();
            assert!(count > 0);
            offset += count;
        }
        stdin.close();
        assert!(matches!(
            stdin.write(b"after EOF").await,
            Err(ProcessError::Io(std::io::ErrorKind::BrokenPipe))
        ));
    });
    assert_eq!(output.0, bytes);
    assert!(output.1.is_empty());
    assert_eq!(process.exit().unwrap().code, Some(0));
}
#[tokio::test]
async fn explicit_environment_and_directory_do_not_inherit_cargo_environment() {
    let temp = tempfile::tempdir().unwrap();
    let mut input = spec(
        temp.path(),
        "test -z \"${CARGO_MANIFEST_DIR+x}\" && test \"$AGENT_TRANSPORT_TEST\" = present && test -f marker && printf passed",
    );
    std::fs::write(temp.path().join("marker"), "fixture").unwrap();
    input
        .environment
        .insert("AGENT_TRANSPORT_TEST".into(), "present".into());
    let (mut process, stdin) = PipeProcess::spawn(input).unwrap();
    drop(stdin);
    let (stdout, _) = drain(&mut process).await;
    assert!(stdout == b"passed");
    assert_eq!(process.exit().unwrap().code, Some(0));
}
#[tokio::test]
async fn interrupt_delivers_sigint_and_termination_retains_signal_status() {
    let temp = tempfile::tempdir().unwrap();
    let (mut process, stdin) = PipeProcess::spawn(spec(
        temp.path(),
        "trap 'printf interrupted; exit 7' INT; printf ready; while :; do /bin/sleep 1; done",
    ))
    .unwrap();
    drop(stdin);
    assert_eq!(
        timeout(Duration::from_secs(5), process.next_output())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .bytes,
        b"ready"
    );
    process.interrupt().unwrap();
    let (stdout, _) = drain(&mut process).await;
    assert_eq!(stdout, b"interrupted");
    assert_eq!(process.exit().unwrap().code, Some(7));
    let (mut process, stdin) = PipeProcess::spawn(spec(temp.path(), "exec /bin/sleep 60")).unwrap();
    drop(stdin);
    process.terminate().unwrap();
    drain(&mut process).await;
    assert_eq!(
        process.exit(),
        Some(ProcessExit {
            code: None,
            signal: Some(9)
        })
    );
    process.terminate().unwrap();
}
#[tokio::test]
async fn owner_drop_and_root_exit_do_not_leave_group_descendants_running() {
    let temp = tempfile::tempdir().unwrap();
    for drop_owner in [true, false] {
        let marker = temp
            .path()
            .join(if drop_owner { "drop-leak" } else { "exit-leak" });
        let script = if drop_owner {
            "(/bin/sleep 0.3; printf leaked > \"$1\") & printf ready; wait"
        } else {
            "(/bin/sleep 0.3; printf leaked > \"$1\") & printf ready; exit 0"
        };
        let mut input = spec(temp.path(), script);
        input.arguments.push(marker.clone().into_os_string());
        let (mut process, stdin) = PipeProcess::spawn(input).unwrap();
        drop(stdin);
        if drop_owner {
            let output = timeout(Duration::from_secs(5), process.next_output())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(output.bytes, b"ready");
            drop(process);
        } else {
            assert_eq!(drain(&mut process).await.0, b"ready");
            assert_eq!(process.exit().unwrap().code, Some(0));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            !marker.exists(),
            "command descendant escaped lifecycle cleanup"
        );
    }
}
#[tokio::test]
async fn spawn_failures_are_typed_and_do_not_echo_command_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let mut input = spec(temp.path(), "do not echo this argument");
    input.program = "relative-program".into();
    assert!(matches!(
        PipeProcess::spawn(input),
        Err(ProcessError::Invalid)
    ));
    let mut input = spec(temp.path(), "do not echo this argument");
    input.program = temp.path().join("missing-program");
    let Err(error) = PipeProcess::spawn(input) else {
        panic!()
    };
    assert!(matches!(
        error,
        ProcessError::Io(std::io::ErrorKind::NotFound)
    ));
    assert!(!error.to_string().contains("argument"));
}

#[tokio::test]
async fn pipe_command_has_an_independent_session_and_process_group() {
    let temp = tempfile::tempdir().unwrap();
    let (mut process, stdin) =
        PipeProcess::spawn(spec(temp.path(), "printf '%s' \"$$\"; exec /bin/sleep 60")).unwrap();
    drop(stdin);
    let output = timeout(Duration::from_secs(5), process.next_output())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let pid = Pid::from_raw(std::str::from_utf8(&output.bytes).unwrap().parse().unwrap());
    assert_eq!(nix::unistd::getsid(Some(pid)).unwrap(), pid);
    assert_eq!(nix::unistd::getpgid(Some(pid)).unwrap(), pid);
    assert_ne!(nix::unistd::getsid(None).unwrap(), pid);
    process.terminate().unwrap();
    drain(&mut process).await;
}
