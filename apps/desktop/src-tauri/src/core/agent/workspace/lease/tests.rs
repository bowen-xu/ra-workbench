use super::*;
use research_agent_core::workspace::{BaselineFile, FileChangeKind, SourceVersion, WorkspaceFile};
use std::{
    fs,
    os::unix::fs::{MetadataExt, symlink},
};
use uuid::Uuid;
fn manifest() -> WorkspaceManifest {
    WorkspaceManifest {
        schema_version: 1,
        identity: WorkspaceIdentity {
            lease_id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            environment_id: "manuscript:test".into(),
        },
        files: vec![BaselineFile {
            file: WorkspaceFile {
                path: "sections/main.tex".into(),
                version: tree::version(b"user buffer", false),
            },
            source: SourceVersion {
                resource_id: "manuscript:test/main".into(),
                revision: "disk:before-buffer".into(),
                editor: None,
            },
        }],
    }
}
fn input() -> Vec<WorkspaceInput> {
    vec![WorkspaceInput {
        path: "sections/main.tex".into(),
        bytes: b"user buffer".to_vec(),
    }]
}
fn store(path: &Path) -> WorkspaceStore {
    WorkspaceStore::open(&path.canonicalize().unwrap(), |_| Ok(())).unwrap()
}
#[path = "tests/discard_receipts.rs"]
mod discard_receipts;

#[test]
fn delivery_handoff_preserves_the_reader_lock_and_disarms_the_old_writer() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let mut writer = store.create(manifest.clone(), input()).unwrap();
    assert!(matches!(
        store.restore(&manifest.identity),
        Err(LeaseError::Busy)
    ));
    writer.release_for_delivery().unwrap();
    assert!(matches!(
        writer.begin_execution(),
        Err(LeaseError::Unauthorized)
    ));
    assert_eq!(
        writer.apply_text_operations(&[TextPatchOperation {
            path: "x.tex".into(),
            before: None,
            after: Some("x".into())
        }]),
        Err(LeaseError::Unauthorized)
    );
    let reader = store.restore(&manifest.identity).unwrap();
    drop(writer);
    assert!(matches!(
        store.restore(&manifest.identity),
        Err(LeaseError::Busy)
    ));
    drop(reader);
    assert!(store.restore(&manifest.identity).is_ok());
}
#[path = "tests/review.rs"]
mod review;
#[test]
fn duplicated_descriptor_does_not_retain_released_application_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    // dup shares the same open-file description, as an inherited descriptor does.
    let inherited = lease._lock.0.try_clone().unwrap();
    assert!(matches!(
        store.restore(&manifest.identity),
        Err(LeaseError::Busy)
    ));
    drop(lease);
    let restored = store.restore(&manifest.identity).unwrap();
    drop(restored);
    store.discard(&manifest.identity).unwrap();
    drop(inherited);
}

#[test]
fn creates_independent_files_and_recovers_changes_under_exclusive_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    assert!(lease.changes().unwrap().is_empty());
    assert!(matches!(
        store.restore(&manifest.identity),
        Err(LeaseError::Busy)
    ));
    assert_eq!(store.discard(&manifest.identity), Err(LeaseError::Busy));
    let path = lease.working_directory().unwrap();
    fs::write(path.join("sections/main.tex"), "agent edit").unwrap();
    fs::write(path.join("new.bin"), [0, 255, 1]).unwrap();
    assert_eq!(
        fs::read(path.parent().unwrap().join("base/sections/main.tex")).unwrap(),
        b"user buffer"
    );
    let changes = lease.changes().unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[1].kind, FileChangeKind::Modified);
    assert_eq!(
        changes[1].source.as_ref().unwrap().revision,
        "disk:before-buffer"
    );
    drop(lease);
    let restored = retry_busy(|| store.restore(&manifest.identity)).unwrap();
    assert_eq!(restored.manifest(), &manifest);
    assert_eq!(restored.changes().unwrap(), changes);
    fs::remove_file(path.join("sections/main.tex")).unwrap();
    assert_eq!(restored.changes().unwrap()[1].kind, FileChangeKind::Deleted);
    drop(restored);
    retry_busy(|| store.discard(&manifest.identity)).unwrap();
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn native_text_patch_checks_before_bytes_and_reports_diff() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let mut lease = store.create(manifest, input()).unwrap();
    let changes = lease
        .apply_text_operations(&[TextPatchOperation {
            path: "sections/main.tex".into(),
            before: Some("user buffer".into()),
            after: Some("agent edit".into()),
        }])
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, FileChangeKind::Modified);
    assert_eq!(
        fs::read(lease.working_directory().unwrap().join("sections/main.tex")).unwrap(),
        b"agent edit"
    );
    assert_eq!(
        lease.apply_text_operations(&[TextPatchOperation {
            path: "sections/main.tex".into(),
            before: Some("stale".into()),
            after: Some("ignored".into()),
        }]),
        Err(LeaseError::ContentMismatch)
    );
}

#[test]
fn native_text_patch_accepts_one_unique_context_hunk() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    let path = lease.working_directory().unwrap().join("sections/main.tex");
    fs::write(&path, "prefix\\nuser buffer\\nsuffix\\n").unwrap();
    drop(lease);
    let mut lease = store.restore(&manifest.identity).unwrap();
    let changes = lease
        .apply_text_operations(&[TextPatchOperation {
            path: "sections/main.tex".into(),
            before: Some("user buffer".into()),
            after: Some("agent edit".into()),
        }])
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "prefix\\nagent edit\\nsuffix\\n"
    );
    fs::write(&path, "agent edit\\nagent edit\\n").unwrap();
    assert_eq!(
        lease.apply_text_operations(&[TextPatchOperation {
            path: "sections/main.tex".into(),
            before: Some("agent edit".into()),
            after: Some("replacement".into()),
        }]),
        Err(LeaseError::AmbiguousPatch)
    );
}

#[test]
fn visits_current_work_bytes_and_rejects_late_changes() {
    let temp = tempfile::tempdir().unwrap();
    let lease = store(temp.path()).create(manifest(), input()).unwrap();
    let work = lease.working_directory().unwrap();
    fs::write(work.join("sections/main.tex"), b"compiled source").unwrap();
    let mut seen = vec![];
    lease
        .visit_work_bytes(|file, bytes| {
            seen.push((file.path.clone(), bytes.to_vec()));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        seen,
        vec![("sections/main.tex".into(), b"compiled source".to_vec())]
    );
    let mut mutated = false;
    let error = lease.visit_work_bytes(|_, _| {
        if !mutated {
            fs::write(work.join("sections/main.tex"), b"changed during scan").unwrap();
            mutated = true;
        }
        Ok(())
    });
    assert_eq!(error, Err(LeaseError::Conflict));
}
#[test]
fn failed_preparation_rolls_back_only_its_new_directory() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let mut bad = input();
    bad[0].bytes = b"different".to_vec();
    assert!(matches!(
        store.create(manifest.clone(), bad),
        Err(LeaseError::Conflict)
    ));
    assert!(
        !temp
            .path()
            .join(manifest.identity.lease_id.to_string())
            .exists()
    );
    assert!(matches!(
        store.create(manifest.clone(), vec![]),
        Err(LeaseError::Incomplete)
    ));
    let lease = store.create(manifest.clone(), input()).unwrap();
    assert!(matches!(
        store.create(manifest.clone(), input()),
        Err(LeaseError::Conflict)
    ));
    assert!(lease.working_directory().unwrap().exists());
}
#[test]
fn rejects_wrong_identity_tampered_baseline_and_incomplete_preparation() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    let path = lease.working_directory().unwrap();
    drop(lease);
    let mut other = manifest.identity.clone();
    other.turn_id = Uuid::new_v4();
    assert!(matches!(
        retry_busy(|| store.restore(&other)),
        Err(LeaseError::Conflict)
    ));
    assert_eq!(
        retry_busy(|| store.discard(&other)),
        Err(LeaseError::Conflict)
    );
    fs::write(
        path.parent().unwrap().join("base/sections/main.tex"),
        "changed",
    )
    .unwrap();
    assert!(matches!(
        retry_busy(|| store.restore(&manifest.identity)),
        Err(LeaseError::Conflict)
    ));
    fs::remove_file(path.parent().unwrap().join("ready")).unwrap();
    assert!(matches!(
        retry_busy(|| store.restore(&manifest.identity)),
        Err(LeaseError::Incomplete)
    ));
    retry_busy(|| store.discard(&manifest.identity)).unwrap();
}
#[test]
fn work_links_special_files_and_oversize_files_are_rejected_but_cleanup_never_follows_links() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    let path = lease.working_directory().unwrap();
    fs::write(outside.path().join("keep"), "external").unwrap();
    symlink(outside.path(), path.join("link")).unwrap();
    assert_eq!(lease.changes(), Err(LeaseError::Invalid));
    fs::remove_file(path.join("link")).unwrap();
    nix::unistd::mkfifo(&path.join("fifo"), nix::sys::stat::Mode::S_IRUSR).unwrap();
    assert_eq!(lease.changes(), Err(LeaseError::Invalid));
    fs::remove_file(path.join("fifo")).unwrap();
    let huge = fs::File::create(path.join("huge")).unwrap();
    huge.set_len(research_agent_core::workspace::MAX_FILE_BYTES + 1)
        .unwrap();
    assert_eq!(lease.changes(), Err(LeaseError::Budget));
    fs::remove_file(path.join("huge")).unwrap();
    symlink(outside.path(), path.join("link")).unwrap();
    // APFS rejects non-UTF-8 names at creation. Exercise raw-name cleanup on
    // Linux filesystems that admit them, without treating rejection as success.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        fs::write(
            path.join(std::ffi::OsString::from_vec(vec![0xff])),
            b"invalid name",
        )
        .unwrap();
    }
    drop(lease);
    retry_busy(|| store.discard(&manifest.identity)).unwrap();
    assert_eq!(fs::read(outside.path().join("keep")).unwrap(), b"external");
}
#[test]
fn opened_storage_does_not_follow_a_replaced_root_path() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("storage");
    fs::create_dir(&original).unwrap();
    let original = original.canonicalize().unwrap();
    let moved = temp.path().join("pinned");
    let store = WorkspaceStore::open(&original, |_| {
        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        fs::write(original.join("keep"), "replacement").unwrap();
        Ok(())
    })
    .unwrap();
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    assert!(lease.changes().unwrap().is_empty());
    assert!(lease.working_directory().is_err());
    assert!(moved.join(manifest.identity.lease_id.to_string()).exists());
    drop(lease);
    retry_busy(|| store.discard(&manifest.identity)).unwrap();
    assert_eq!(fs::read(original.join("keep")).unwrap(), b"replacement");
}

#[test]
fn parent_deletion_journal_recovers_after_child_metadata_is_removed() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    let path = lease.working_directory().unwrap();
    drop(lease);
    let metadata = fs::metadata(path.parent().unwrap()).unwrap();
    let journal = temp
        .path()
        .join(super::super::discard::journal_name(&manifest.identity));
    fs::write(&journal,serde_json::to_vec(&serde_json::json!({"identity":manifest.identity,"device":metadata.dev(),"inode":metadata.ino()})).unwrap()).unwrap();
    fs::remove_file(path.parent().unwrap().join("manifest.json")).unwrap();
    assert!(matches!(
        retry_busy(|| store.restore(&manifest.identity)),
        Err(LeaseError::Incomplete)
    ));
    let mut wrong = manifest.identity.clone();
    wrong.turn_id = Uuid::new_v4();
    assert_eq!(
        retry_busy(|| store.discard(&wrong)),
        Err(LeaseError::Conflict)
    );
    assert!(path.join("sections/main.tex").exists());
    retry_busy(|| store.discard(&manifest.identity)).unwrap();
    assert!(!path.parent().unwrap().exists());
    assert!(!journal.exists());
}
#[test]
fn deletion_journal_cannot_delete_a_replacement_directory() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    let path = lease.working_directory().unwrap();
    drop(lease);
    let parent = path.parent().unwrap();
    let metadata = fs::metadata(parent).unwrap();
    fs::write(temp.path().join(super::super::discard::journal_name(&manifest.identity)),serde_json::to_vec(&serde_json::json!({"identity":manifest.identity,"device":metadata.dev(),"inode":metadata.ino()})).unwrap()).unwrap();
    fs::rename(parent, temp.path().join("moved-original")).unwrap();
    fs::create_dir(parent).unwrap();
    fs::write(parent.join("keep"), "replacement").unwrap();
    assert_eq!(
        retry_busy(|| store.discard(&manifest.identity)),
        Err(LeaseError::Conflict)
    );
    assert_eq!(fs::read(parent.join("keep")).unwrap(), b"replacement");
}
#[test]
fn hard_links_are_not_collected_and_filesystem_aliases_cannot_publish_a_wrong_baseline() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let original = manifest();
    let lease = store.create(original.clone(), input()).unwrap();
    let path = lease.working_directory().unwrap();
    fs::hard_link(path.join("sections/main.tex"), path.join("hardlink")).unwrap();
    assert_eq!(lease.changes(), Err(LeaseError::Invalid));
    drop(lease);
    retry_busy(|| store.discard(&original.identity)).unwrap();
    let mut aliased = manifest();
    aliased.files[0].file.path = "A.tex".into();
    let mut second = aliased.files[0].clone();
    second.file.path = "a.tex".into();
    aliased.files.push(second);
    let copies = vec![
        WorkspaceInput {
            path: "A.tex".into(),
            bytes: b"user buffer".to_vec(),
        },
        WorkspaceInput {
            path: "a.tex".into(),
            bytes: b"user buffer".to_vec(),
        },
    ];
    match store.create(aliased.clone(), copies) {
        Ok(lease) => assert!(lease.changes().unwrap().is_empty()), // case-sensitive filesystem
        Err(LeaseError::Conflict) => assert!(
            !temp
                .path()
                .join(aliased.identity.lease_id.to_string())
                .exists()
        ),
        Err(error) => panic!("unexpected alias result: {error}"),
    }
}

#[test]
fn unauthorized_storage_and_replaced_live_worktree_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    assert!(matches!(
        WorkspaceStore::open(&root, |_| Err(LeaseError::Unauthorized)),
        Err(LeaseError::Unauthorized)
    ));
    let store = store(&root);
    let manifest = manifest();
    let lease = store.create(manifest.clone(), input()).unwrap();
    let work = lease.working_directory().unwrap();
    fs::rename(&work, work.with_file_name("moved-work")).unwrap();
    fs::create_dir(&work).unwrap();
    assert_eq!(lease.changes(), Err(LeaseError::Conflict));
    assert_eq!(lease.working_directory(), Err(LeaseError::Conflict));
    drop(lease);
    retry_busy(|| store.discard(&manifest.identity)).unwrap();
}

#[tokio::test]
async fn execution_guard_blocks_scan_and_cleanup_until_real_process_is_reaped() {
    use crate::agent::process::{ProcessLimits, ProcessSpec};
    use std::time::Duration;
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let mut lease = store.create(manifest.clone(), input()).unwrap();
    let guard = lease.begin_execution().unwrap();
    assert_eq!(lease.changes(), Err(LeaseError::Busy));
    assert_eq!(lease.release_for_delivery(), Err(LeaseError::Busy));
    assert!(matches!(lease.begin_execution(), Err(LeaseError::Busy)));
    let working_directory = guard.working_directory().to_path_buf();
    let mut session = guard
        .spawn(
            ProcessSpec {
                program: "/bin/sh".into(),
                arguments: vec!["-c".into(), "read line; printf edit > new.tex".into()],
                directory: working_directory,
                environment: BTreeMap::new(),
            },
            ProcessLimits {
                retained_bytes: 1024,
                execution_timeout: Duration::from_secs(5),
                drain_timeout: Duration::from_secs(2),
            },
            None,
        )
        .unwrap();
    drop(lease);
    assert_eq!(store.discard(&manifest.identity), Err(LeaseError::Busy));
    session.write(b"\n").await.unwrap();
    let result = session.finish().await.unwrap();
    assert!(result.resource_error.is_none());
    let lease = retry_busy(|| store.restore(&manifest.identity)).unwrap();
    assert_eq!(lease.changes().unwrap()[0].kind, FileChangeKind::Added);
}
#[test]
fn abandoned_execution_remains_unrestorable_and_undeletable_after_lock_release() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let mut lease = store.create(manifest.clone(), input()).unwrap();
    let guard = lease.begin_execution().unwrap();
    drop(lease);
    assert!(matches!(
        store.restore(&manifest.identity),
        Err(LeaseError::Busy)
    ));
    drop(guard);
    assert!(matches!(
        retry_busy(|| store.restore(&manifest.identity)),
        Err(LeaseError::Incomplete)
    ));
    assert_eq!(
        retry_busy(|| store.discard(&manifest.identity)),
        Err(LeaseError::Incomplete)
    );
    assert!(
        temp.path()
            .join(manifest.identity.lease_id.to_string())
            .join("execution.json")
            .exists()
    );
}
#[tokio::test]
async fn incomplete_reaping_does_not_release_writer_intent() {
    use crate::agent::process::{ProcessLimits, ProcessSpec};
    use std::time::Duration;
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let manifest = manifest();
    let mut lease = store.create(manifest.clone(), input()).unwrap();
    let guard = lease.begin_execution().unwrap();
    let working_directory = guard.working_directory().to_path_buf();
    let mut session = guard
        .spawn(
            ProcessSpec {
                program: "/bin/sleep".into(),
                arguments: vec!["30".into()],
                directory: working_directory,
                environment: BTreeMap::new(),
            },
            ProcessLimits {
                retained_bytes: 1024,
                execution_timeout: Duration::from_secs(5),
                drain_timeout: Duration::from_nanos(1),
            },
            None,
        )
        .unwrap();
    let result = session.shutdown().await.unwrap();
    assert!(!result.drained);
    assert!(result.resource_error.is_some());
    assert_eq!(lease.changes(), Err(LeaseError::Busy));
    drop(lease);
    assert_eq!(
        retry_busy(|| store.discard(&manifest.identity)),
        Err(LeaseError::Incomplete)
    );
}

// Parallel fork briefly inherits another test's CLOEXEC lock until exec. Only
// retry that transient native lock state; never hide identity/storage failures.
fn retry_busy<T>(mut operation: impl FnMut() -> Result<T, LeaseError>) -> Result<T, LeaseError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match operation() {
            Err(LeaseError::Busy) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            result => return result,
        }
    }
}

#[path = "tests/change_bytes.rs"]
mod change_bytes;
