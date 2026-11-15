use super::*;
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio::time::timeout;
fn spec(path: &Path, script: &str) -> ProcessSpec {
    ProcessSpec {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), script.into()],
        directory: path.into(),
        environment: BTreeMap::new(),
    }
}
fn size() -> TerminalSize {
    TerminalSize {
        rows: 24,
        columns: 80,
    }
}
async fn until(process: &mut PtyProcess, marker: &[u8]) -> Vec<u8> {
    timeout(Duration::from_secs(5), async {
        let mut bytes = vec![];
        loop {
            let chunk = process
                .next_output()
                .await
                .unwrap()
                .expect("premature terminal EOF");
            bytes.extend(chunk);
            if bytes.windows(marker.len()).any(|window| window == marker) {
                return bytes;
            }
        }
    })
    .await
    .unwrap()
}
async fn drain(process: &mut PtyProcess) -> Vec<u8> {
    timeout(Duration::from_secs(5), async {
        let mut bytes = vec![];
        while let Some(chunk) = process.next_output().await.unwrap() {
            bytes.extend(chunk);
        }
        bytes
    })
    .await
    .unwrap()
}
async fn write(input: &mut PtyInput, bytes: &[u8]) {
    let mut offset = 0;
    while offset < bytes.len() {
        let count = input.write(&bytes[offset..]).await.unwrap();
        assert!(count > 0);
        offset += count;
    }
}
#[tokio::test]
async fn all_stdio_are_terminal_and_resize_reaches_child() {
    let temp = tempfile::tempdir().unwrap();
    let (mut process, mut input) = PtyProcess::spawn(spec(temp.path(),
        "test -t 0 && test -t 1 && test -t 2 || exit 9; /bin/stty -echo; /bin/stty size; printf READY; read line; /bin/stty size; printf 'got:%s' \"$line\""), size()).unwrap();
    let first = until(&mut process, b"READY").await;
    assert!(String::from_utf8_lossy(&first).contains("24 80"));
    assert!(matches!(
        process.resize(TerminalSize {
            rows: 0,
            columns: 120
        }),
        Err(ProcessError::Invalid)
    ));
    process
        .resize(TerminalSize {
            rows: 40,
            columns: 120,
        })
        .unwrap();
    write(&mut input, b"hello\n").await;
    let rest = drain(&mut process).await;
    assert!(String::from_utf8_lossy(&rest).contains("40 120"));
    assert!(String::from_utf8_lossy(&rest).contains("got:hello"));
    assert_eq!(process.exit().unwrap().code, Some(0));
}
#[tokio::test]
async fn cancelled_read_can_resume_and_terminal_eof_is_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let (mut process, mut input) = PtyProcess::spawn(
        spec(temp.path(), "/bin/stty -echo; printf READY; exec /bin/cat"),
        size(),
    )
    .unwrap();
    until(&mut process, b"READY").await;
    assert!(
        timeout(Duration::from_millis(25), process.next_output())
            .await
            .is_err()
    );
    write(&mut input, b"copied\n").await;
    until(&mut process, b"copied").await;
    write(&mut input, &[4]).await;
    drain(&mut process).await;
    assert_eq!(process.exit().unwrap().code, Some(0));
    input.close();
    assert!(matches!(
        input.write(b"closed").await,
        Err(ProcessError::Io(std::io::ErrorKind::BrokenPipe))
    ));
}
#[tokio::test]
async fn terminal_ctrl_c_delivers_sigint_and_reaps_child() {
    let temp = tempfile::tempdir().unwrap();
    let (mut process, mut input) = PtyProcess::spawn(
        spec(
            temp.path(),
            "/bin/stty -echo; printf READY; exec /bin/sleep 60",
        ),
        size(),
    )
    .unwrap();
    until(&mut process, b"READY").await;
    write(&mut input, &[3]).await;
    drain(&mut process).await;
    assert_eq!(process.exit().unwrap().signal, Some(nix::libc::SIGINT));
}
#[tokio::test]
async fn preserves_large_combined_binary_output_and_final_status() {
    let temp = tempfile::tempdir().unwrap();
    let (mut process, _input) = PtyProcess::spawn(spec(temp.path(),
        "printf OUT; printf ERR >&2; /bin/dd if=/dev/zero bs=8192 count=20 2>/dev/null; printf END; exit 7"), size()).unwrap();
    let bytes = drain(&mut process).await;
    assert_eq!(&bytes[..6], b"OUTERR");
    assert_eq!(&bytes[6..bytes.len() - 3], vec![0; 8192 * 20]);
    assert_eq!(&bytes[bytes.len() - 3..], b"END");
    assert_eq!(process.exit().unwrap().code, Some(7));
}
#[cfg(target_os = "macos")]
#[tokio::test]
async fn offline_sandbox_retains_terminal_behavior_and_denies_reserved_files() {
    use crate::agent::process::{OfflineSandboxPaths, prepare_offline_sandbox};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let work = root.join("work");
    let reserved = root.join("reserved");
    std::fs::create_dir(&work).unwrap();
    std::fs::create_dir(&reserved).unwrap();
    std::fs::write(reserved.join("fixture"), "private-fixture").unwrap();
    let mut command = spec(
        &work,
        "/bin/stty -echo; printf READY; read line; if /bin/cat \"$1\" >/dev/null 2>/dev/null; then exit 8; fi; printf 'got:%s' \"$line\"",
    );
    command.arguments.extend([
        "fixture-command".into(),
        reserved.join("fixture").into_os_string(),
    ]);
    let command = prepare_offline_sandbox(
        command,
        OfflineSandboxPaths {
            work,
            read_only: vec![],
            protected: vec![reserved],
            allow_system_metadata: false,
        },
    )
    .unwrap();
    let (mut process, mut input) = PtyProcess::spawn(command, size()).unwrap();
    let first = until(&mut process, b"READY").await;
    assert!(!String::from_utf8_lossy(&first).contains("not permitted"));
    write(&mut input, b"sandboxed\n").await;
    assert!(String::from_utf8_lossy(&drain(&mut process).await).contains("got:sandboxed"));
    assert_eq!(process.exit().unwrap().code, Some(0));
}
