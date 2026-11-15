use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
#[path = "tests/mutate.rs"]
mod mutate;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("agent-disk-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn version(bytes: &[u8], executable: bool) -> FileVersion {
    FileVersion {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
        executable,
    }
}

#[test]
fn publishes_complete_binary_and_mode_without_overwriting_existing_files() {
    let f = Fixture::new();
    std::fs::create_dir(f.0.join("assets")).unwrap();
    let target = DiskTarget::open(&f.0, "assets/data.bin").unwrap();
    let bytes = b"\x00\xff\r\nbody";
    let expected = version(bytes, true);
    assert_eq!(target.observe().unwrap(), None);
    assert_eq!(
        target.create(Uuid::new_v4(), bytes, &expected).unwrap(),
        expected
    );
    assert_eq!(target.observe().unwrap(), Some(expected.clone()));
    assert_eq!(std::fs::read(f.0.join("assets/data.bin")).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(f.0.join("assets/data.bin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        target.create(
            Uuid::new_v4(),
            b"replacement",
            &version(b"replacement", false)
        ),
        Err(DiskError::Conflict)
    );
    assert_eq!(target.observe().unwrap(), Some(expected));
    assert_eq!(std::fs::read_dir(f.0.join("assets")).unwrap().count(), 1);
}

#[test]
fn refuses_unsafe_paths_occupied_names_and_replaced_parents() {
    let f = Fixture::new();
    assert!(DiskTarget::open(&f.0, "../escape").is_err());
    std::fs::create_dir(f.0.join("parent")).unwrap();
    let target = DiskTarget::open(&f.0, "parent/new.tex").unwrap();
    std::fs::rename(f.0.join("parent"), f.0.join("moved")).unwrap();
    std::fs::create_dir(f.0.join("parent")).unwrap();
    assert_eq!(
        target.create(Uuid::new_v4(), b"x", &version(b"x", false)),
        Err(DiskError::Conflict)
    );
    assert!(!f.0.join("moved/new.tex").exists());
    assert!(!f.0.join("parent/new.tex").exists());
    symlink(f.0.join("parent"), f.0.join("alias")).unwrap();
    assert!(DiskTarget::open(&f.0, "alias/new.tex").is_err());
    symlink(f.0.join("missing"), f.0.join("link.tex")).unwrap();
    let target = DiskTarget::open(&f.0, "link.tex").unwrap();
    assert!(target.observe().is_err());
    assert_eq!(
        target.create(Uuid::new_v4(), b"x", &version(b"x", false)),
        Err(DiskError::Conflict)
    );
    assert!(
        std::fs::symlink_metadata(f.0.join("link.tex"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let target = DiskTarget::open(&f.0, "parent").unwrap();
    assert!(target.observe().is_err());
    assert_eq!(
        target.create(Uuid::new_v4(), b"x", &version(b"x", false)),
        Err(DiskError::Conflict)
    );
}

#[test]
fn validates_frozen_bytes_before_any_mutation_and_preserves_existing_stage() {
    let f = Fixture::new();
    let target = DiskTarget::open(&f.0, "new.tex").unwrap();
    assert_eq!(
        target.create(Uuid::new_v4(), b"x", &version(b"wrong", false)),
        Err(DiskError::Invalid)
    );
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 0);
    let operation = Uuid::new_v4();
    let stage = f.0.join(format!(
        ".agent-create-{operation}-{:x}.tmp",
        Sha256::digest(b"new.tex")
    ));
    std::fs::write(&stage, b"old attempt").unwrap();
    assert_eq!(
        target.create(operation, b"x", &version(b"x", false)),
        Err(DiskError::RecoveryRequired)
    );
    assert_eq!(std::fs::read(stage).unwrap(), b"old attempt");
    assert!(!f.0.join("new.tex").exists());
}

#[test]
fn observes_empty_files_but_rejects_hard_links() {
    let f = Fixture::new();
    let target = DiskTarget::open(&f.0, "empty.tex").unwrap();
    target
        .create(Uuid::new_v4(), b"", &version(b"", false))
        .unwrap();
    assert_eq!(target.observe().unwrap(), Some(version(b"", false)));
    std::fs::hard_link(f.0.join("empty.tex"), f.0.join("alias.tex")).unwrap();
    assert_eq!(target.observe(), Err(DiskError::Conflict));
}

#[test]
fn reconciles_only_the_verified_redundant_publication_link() {
    let f = Fixture::new();
    let target = DiskTarget::open(&f.0, "new.tex").unwrap();
    let operation = Uuid::new_v4();
    let stage = f.0.join(target.temporary_name(operation));
    std::fs::write(&stage, b"complete").unwrap();
    // A stage without a published target is never automatically applied.
    assert!(
        target
            .reconcile_create(operation, &version(b"complete", false))
            .is_err()
    );
    assert!(stage.exists());
    assert!(!f.0.join("new.tex").exists());
    std::fs::hard_link(&stage, f.0.join("new.tex")).unwrap();
    assert!(target.observe().is_err());
    assert_eq!(
        target
            .reconcile_create(operation, &version(b"complete", false))
            .unwrap(),
        version(b"complete", false)
    );
    assert!(!stage.exists());
    assert_eq!(std::fs::read(f.0.join("new.tex")).unwrap(), b"complete");
    assert_eq!(
        target
            .reconcile_create(operation, &version(b"complete", false))
            .unwrap(),
        version(b"complete", false)
    );
}

#[test]
fn preserves_unrelated_or_modified_staging_artifacts() {
    let f = Fixture::new();
    let target = DiskTarget::open(&f.0, "new.tex").unwrap();
    let operation = Uuid::new_v4();
    let stage = f.0.join(target.temporary_name(operation));
    let destination = f.0.join("new.tex");
    std::fs::write(&stage, b"complete").unwrap();
    std::fs::write(&destination, b"complete").unwrap();
    assert!(
        target
            .reconcile_create(operation, &version(b"complete", false))
            .is_err()
    );
    assert!(stage.exists());
    std::fs::remove_file(&destination).unwrap();
    std::fs::hard_link(&stage, &destination).unwrap();
    std::fs::write(&destination, b"user change").unwrap();
    assert!(
        target
            .reconcile_create(operation, &version(b"complete", false))
            .is_err()
    );
    assert_eq!(std::fs::read(&stage).unwrap(), b"user change");
    assert_eq!(std::fs::read(&destination).unwrap(), b"user change");
}
