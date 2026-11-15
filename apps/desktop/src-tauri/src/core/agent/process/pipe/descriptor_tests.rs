use super::*;
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

fn inheritable(fd: &impl std::os::fd::AsFd, minimum: i32) -> OwnedFd {
    let fd = nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD(minimum)).unwrap();
    // SAFETY: F_DUPFD returned a newly owned descriptor; this is its sole owner.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    assert_eq!(
        nix::fcntl::fcntl(&owned, nix::fcntl::FcntlArg::F_GETFD).unwrap(),
        0
    );
    owned
}
async fn output(process: &mut PipeProcess) -> Vec<u8> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut bytes = vec![];
        while let Some(chunk) = process.next_output().await.unwrap() {
            if chunk.stream == OutputStream::Stdout {
                bytes.extend(chunk.bytes);
            }
        }
        bytes
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn inherited_file_is_closed_only_in_child_and_spawn_error_channel_survives() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("fixture");
    std::fs::write(&path, "fixture-owned").unwrap();
    let file = File::open(path).unwrap();
    let duplicate = inheritable(&file, 512);
    let fd_path = format!("/dev/fd/{}", duplicate.as_raw_fd());
    // Positive control: std's default launch passes this intentionally non-CLOEXEC
    // descriptor. The negative check below must therefore test our own boundary.
    let control = std::process::Command::new("/bin/cat")
        .arg(&fd_path)
        .output()
        .unwrap();
    assert!(control.status.success());
    assert_eq!(control.stdout, b"fixture-owned");
    let (mut process, input) = PipeProcess::spawn(ProcessSpec {
        program: "/bin/cat".into(),
        arguments: vec![fd_path.into()],
        directory: temp.path().into(),
        environment: BTreeMap::new(),
    })
    .unwrap();
    drop(input);
    assert!(output(&mut process).await.is_empty());
    assert_ne!(process.exit().unwrap().code, Some(0));
    assert_eq!(
        nix::fcntl::fcntl(&duplicate, nix::fcntl::FcntlArg::F_GETFD).unwrap(),
        0
    );
    let bad = ProcessSpec {
        program: temp.path().join("missing-program"),
        arguments: vec![],
        directory: temp.path().into(),
        environment: BTreeMap::new(),
    };
    assert!(matches!(
        PipeProcess::spawn(bad),
        Err(ProcessError::Io(std::io::ErrorKind::NotFound))
    ));
}

#[tokio::test]
async fn inherited_socket_is_unavailable_to_child_but_parent_can_still_use_it() {
    let (socket, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    let duplicate = inheritable(&socket, 1024);
    let temp = tempfile::tempdir().unwrap();
    let script = format!("printf leaked >&{}", duplicate.as_raw_fd());
    let (mut process, input) = PipeProcess::spawn(ProcessSpec {
        program: "/bin/sh".into(),
        arguments: vec!["-c".into(), script.into()],
        directory: temp.path().into(),
        environment: BTreeMap::new(),
    })
    .unwrap();
    drop(input);
    output(&mut process).await;
    assert_ne!(process.exit().unwrap().code, Some(0));
    peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let mut parent = socket;
    parent.write_all(b"parent").unwrap();
    let mut bytes = [0; 6];
    peer.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"parent");
    assert_eq!(
        nix::fcntl::fcntl(&duplicate, nix::fcntl::FcntlArg::F_GETFD).unwrap(),
        0
    );
}

#[test]
fn descriptor_opened_after_preparation_is_also_marked_for_exec() {
    use std::os::unix::process::CommandExt;
    let plan = super::super::descriptors::DescriptorPlan::prepare().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("late-fixture");
    std::fs::write(&path, "must-not-leak").unwrap();
    let late = inheritable(&File::open(path).unwrap(), 4096);
    let mut command = std::process::Command::new("/bin/cat");
    command.arg(format!("/dev/fd/{}", late.as_raw_fd()));
    // SAFETY: only the same precomputed descriptor syscalls as production; no
    // allocation, locking or parent-owned resources are used after fork.
    unsafe {
        command.pre_exec(move || plan.apply());
    }
    let result = command.output().unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(
        nix::fcntl::fcntl(&late, nix::fcntl::FcntlArg::F_GETFD).unwrap(),
        0
    );
}
