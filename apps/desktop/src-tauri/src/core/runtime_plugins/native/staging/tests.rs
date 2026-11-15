use super::*;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn parent() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
fn canonical(parent: &tempfile::TempDir) -> PathBuf {
    parent.path().canonicalize().unwrap()
}
fn input(session: &StagingSession) -> PathBuf {
    let directory = tempfile::Builder::new()
        .prefix("research-native-input-")
        .tempdir_in(session.path())
        .unwrap()
        .keep();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(directory.join("synthetic-input.json"), b"synthetic only").unwrap();
    directory
}

#[test]
fn live_sessions_are_isolated_and_abandoned_input_is_collected() {
    let parent = parent();
    let parent = canonical(&parent);
    let first = StagingSession::open_in(&parent).unwrap();
    let path = first.path.clone();
    let retained = input(&first);
    let second = StagingSession::open_in(&parent).unwrap();
    assert!(retained.join("synthetic-input.json").exists());
    drop(first);
    assert!(path.exists(), "Drop must retain uncertain input");
    let third = StagingSession::open_in(&parent).unwrap();
    assert!(!path.exists());
    assert!(second.path.exists());
    let second_path = second.path.clone();
    let third_path = third.path.clone();
    drop((second, third));
    assert!(!second_path.exists());
    assert!(!third_path.exists());
}

#[test]
fn catalog_clones_share_owner_and_staged_host_retains_it() {
    let (_root, catalog, key) = super::super::package::tests::literature_fixture(b"host");
    let staged = super::super::package::stage_for(
        &catalog,
        &key,
        super::super::NativeProfile::NoteTraceDecision,
    )
    .unwrap();
    let clone = (*catalog).clone();
    let session = for_catalog(&clone).unwrap();
    let path = session.path.clone();
    assert_eq!(staged.directory.path().parent(), Some(path.as_path()));
    assert_eq!(
        staged.input_directory.as_ref().unwrap().path().parent(),
        Some(path.as_path())
    );
    drop((catalog, clone, session));
    assert!(path.exists());
    drop(staged);
    assert!(!path.exists());
}

#[test]
fn partial_creation_and_unrelated_paths_have_finite_ownership() {
    let temp = parent();
    let parent = canonical(&temp);
    let session = StagingSession::open_in(&parent).unwrap();
    let partial = session
        .root
        .join(format!("session-{}", uuid::Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&partial).unwrap();
    let unrelated = session.root.join("some-other-tool");
    fs::create_dir(&unrelated).unwrap();
    fs::write(unrelated.join("keep"), b"keep").unwrap();
    let legacy = parent.join("research-native-input-legacy");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("keep"), b"keep").unwrap();
    let next = StagingSession::open_in(&parent).unwrap();
    assert!(!partial.exists());
    assert!(unrelated.join("keep").exists());
    assert!(legacy.join("keep").exists());
    drop(next);
}

#[test]
fn namespace_and_session_links_never_authorize_external_deletion() {
    let temp = parent();
    let parent = canonical(&temp);
    let outside = tempfile::tempdir().unwrap();
    let outside = outside.path().canonicalize().unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    symlink(&outside, parent.join(NAMESPACE)).unwrap();
    assert!(StagingSession::open_in(&parent).is_err());
    fs::remove_file(parent.join(NAMESPACE)).unwrap();
    let session = StagingSession::open_in(&parent).unwrap();
    let link = session
        .root
        .join(format!("session-{}", uuid::Uuid::new_v4()));
    symlink(&outside, &link).unwrap();
    let retained = input(&session);
    symlink(&outside, retained.join("external-directory")).unwrap();
    let path = session.path.clone();
    drop(session);
    let next = StagingSession::open_in(&parent).unwrap();
    assert!(!path.exists());
    assert!(link.is_symlink());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
    drop(next);
}

#[test]
fn invalid_owner_latches_recovery_failure_without_claiming_cleanup() {
    for kind in ["symlink", "hardlink", "permissions", "fifo"] {
        let temp = parent();
        let parent = canonical(&temp);
        let session = StagingSession::open_in(&parent).unwrap();
        let retained = input(&session);
        let owner = session.path.join(OWNER);
        drop(session);
        let external = parent.join("external-lock");
        fs::write(&external, b"keep").unwrap();
        fs::set_permissions(&external, fs::Permissions::from_mode(0o600)).unwrap();
        fs::remove_file(&owner).unwrap();
        match kind {
            "symlink" => symlink(&external, &owner).unwrap(),
            "hardlink" => fs::hard_link(&external, &owner).unwrap(),
            "permissions" => {
                fs::write(&owner, b"").unwrap();
                fs::set_permissions(&owner, fs::Permissions::from_mode(0o644)).unwrap();
            }
            "fifo" => nix::unistd::mkfifo(&owner, nix::sys::stat::Mode::S_IRWXU).unwrap(),
            _ => unreachable!(),
        }
        assert!(StagingSession::open_in(&parent).is_err(), "{kind}");
        assert!(retained.join("synthetic-input.json").exists());
        assert_eq!(fs::read(&external).unwrap(), b"keep");
    }
}

#[test]
fn unexpected_nested_tree_preserves_recoverable_owner() {
    let temp = parent();
    let parent = canonical(&temp);
    let session = StagingSession::open_in(&parent).unwrap();
    let retained = input(&session);
    fs::create_dir(retained.join("unexpected-nested-directory")).unwrap();
    let owner = session.path.join(OWNER);
    drop(session);
    assert!(StagingSession::open_in(&parent).is_err());
    assert!(owner.exists());
    assert!(retained.join("synthetic-input.json").exists());
    fs::remove_dir(retained.join("unexpected-nested-directory")).unwrap();
    let next = StagingSession::open_in(&parent).unwrap();
    assert!(!owner.exists());
    drop(next);
}

#[test]
fn partial_removal_failure_keeps_owner_for_the_next_recovery() {
    let temp = parent();
    let parent = canonical(&temp);
    let session = StagingSession::open_in(&parent).unwrap();
    let first = input(&session);
    let blocked = input(&session);
    let path = session.path.clone();
    drop(session);
    validate_contents(&path).unwrap();
    // Model an I/O failure after preflight by making one owned child read-only.
    // This does not change production qualification or depend on entry order.
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(remove_abandoned(&path).is_err());
    assert!(path.join(OWNER).exists());
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
    let recovered = StagingSession::open_in(&parent).unwrap();
    assert!(!path.exists());
    assert!(!first.exists());
    drop(recovered);
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn owner_process_fixture() {
    let Some(parent) = std::env::var_os("RESEARCH_ASSISTANT_STAGING_TEST_PARENT") else {
        return;
    };
    let session = StagingSession::open_in(Path::new(&parent)).unwrap();
    let input = input(&session);
    let executable = tempfile::Builder::new()
        .prefix("research-native-host-")
        .tempdir_in(session.path())
        .unwrap()
        .keep();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(executable.join("synthetic-host"), b"synthetic only").unwrap();
    fs::write(
        Path::new(&parent).join("ready.tmp"),
        session.path.as_os_str().as_encoded_bytes(),
    )
    .unwrap();
    fs::rename(
        Path::new(&parent).join("ready.tmp"),
        Path::new(&parent).join("ready"),
    )
    .unwrap();
    assert!(input.exists());
    loop {
        std::thread::park();
    }
}

#[test]
fn real_process_death_releases_ownership_and_restart_collects_only_dead_session() {
    let temp = parent();
    let parent = canonical(&temp);
    let live = StagingSession::open_in(&parent).unwrap();
    let live_input = input(&live);
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "runtime_plugins::native::staging::tests::owner_process_fixture",
                "--nocapture",
            ])
            .env("RESEARCH_ASSISTANT_STAGING_TEST_PARENT", &parent)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !parent.join("ready").exists() {
        assert!(Instant::now() < deadline, "child did not publish its owner");
        assert!(child.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    let dead_path = PathBuf::from(fs::read_to_string(parent.join("ready")).unwrap());
    let while_alive = StagingSession::open_in(&parent).unwrap();
    assert!(dead_path.exists());
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    assert!(dead_path.exists(), "SIGKILL bypassed Rust destructors");
    let restarted = StagingSession::open_in(&parent).unwrap();
    assert!(!dead_path.exists());
    assert!(live_input.join("synthetic-input.json").exists());
    assert!(while_alive.path.exists());
    drop((restarted, while_alive));
}
