use super::super::{MARKER, PREFIX, directory};
use std::{fs, os::unix::fs::symlink, os::unix::net::UnixListener, path::PathBuf};

fn synthetic_run() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let owner = tempfile::tempdir_in("/tmp").unwrap();
    let parent = owner.path().canonicalize().unwrap();
    let run = parent.join(format!("{PREFIX}safety"));
    fs::create_dir(&run).unwrap();
    (owner, parent, run)
}

#[test]
fn cold_admission_rejects_file_symlinks_before_reading_saved_or_opening_store() {
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("external-sentinel");
    let bytes = b"external file must never become a marker, database, log or object";
    fs::write(&sentinel, bytes).unwrap();

    for relative in [
        "store/research-assistant.sqlite3",
        "store/data-log/raft/segment.log",
        "store/objects/ab/cd/content",
        "configuration/nested/config.json",
        MARKER,
    ] {
        let (_owner, parent, run) = synthetic_run();
        let link = run.join(relative);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&sentinel, &link).unwrap();

        let error = directory(&parent, Some(run.to_str().unwrap())).unwrap_err();
        assert!(
            error.to_string().contains("rejects symlinks"),
            "{relative}: {error}"
        );
        assert_eq!(fs::read(&sentinel).unwrap(), bytes);
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
}

#[test]
fn cold_admission_rejects_nested_directory_symlinks_without_following_them() {
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("external-sentinel");
    fs::write(&sentinel, b"unchanged directory sentinel").unwrap();
    for relative in [
        "store",
        "store/data-log/nested",
        "store/objects/ab/cd",
        "configuration",
        "configuration/nested",
    ] {
        let (_owner, parent, run) = synthetic_run();
        let link = run.join(relative);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(outside.path(), &link).unwrap();

        let error = directory(&parent, Some(run.to_str().unwrap())).unwrap_err();
        assert!(
            error.to_string().contains("rejects symlinks"),
            "{relative}: {error}"
        );
        assert_eq!(
            fs::read(&sentinel).unwrap(),
            b"unchanged directory sentinel"
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
    }
}

#[test]
fn cold_admission_rejects_special_files() {
    let (_owner, parent, run) = synthetic_run();
    let socket_path = run.join("configuration.sock");
    let _socket = UnixListener::bind(&socket_path).unwrap();
    let error = directory(&parent, Some(run.to_str().unwrap())).unwrap_err();
    assert!(error.to_string().contains("rejects special files"));
}

#[test]
fn cold_admission_accepts_only_regular_owned_tree_without_reading_its_files() {
    let (_owner, parent, run) = synthetic_run();
    for relative in [
        "store/research-assistant.sqlite3",
        "store/data-log/raft/segment.log",
        "store/objects/ab/cd/content",
        "configuration/nested/config.json",
        MARKER,
    ] {
        let path = run.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"regular synthetic bytes").unwrap();
    }
    assert_eq!(
        directory(&parent, Some(run.to_str().unwrap())).unwrap(),
        run
    );
}
