use super::*;
use raft_engine::env::{DefaultFileSystem, FileSystem, Handle, Permission, WriteExt};
use std::io::{Seek, SeekFrom, Write};

fn setup(root: &Path) -> ExtentFileSystem<DefaultFileSystem> {
    ExtentFileSystem::new(
        Arc::new(DefaultFileSystem),
        Account::inventory(root, 1024 * 1024, 128 * 1024).unwrap(),
        crate::LogIoCounters::default(),
    )
}
#[test]
fn unlink_keeps_open_readers_charged_and_rename_does_not_duplicate_extent() {
    let dir = tempfile::tempdir().unwrap();
    let fs = setup(dir.path());
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    let handle = Arc::new(fs.create(&a).unwrap());
    let mut writer = fs.new_writer(handle.clone()).unwrap();
    writer.write_all(&[1; 1024]).unwrap();
    drop(writer);
    fs.rename(&a, &b).unwrap();
    assert!(fs.rename(&a, &dir.path().join("failed")).is_err());
    let reopened = Arc::new(fs.open(&b, Permission::ReadOnly).unwrap());
    assert_eq!(fs.account.usage().unwrap().managed_file_extent_bytes, 1024);
    let reader = fs.new_reader(reopened.clone()).unwrap();
    drop(reopened);
    drop(handle);
    fs.delete(&b).unwrap();
    assert_eq!(fs.account.usage().unwrap().managed_file_extent_bytes, 1024);
    drop(reader);
    assert_eq!(fs.account.usage().unwrap().managed_file_extent_bytes, 0);
}
#[test]
fn truncate_releases_only_after_actual_success_and_reopen_rebuilds_the_account() {
    let dir = tempfile::tempdir().unwrap();
    let fs = setup(dir.path());
    let path = dir.path().join("a");
    let handle = Arc::new(fs.create(&path).unwrap());
    let mut writer = fs.new_writer(handle.clone()).unwrap();
    writer.write_all(&[2; 1024]).unwrap();
    let readonly = fs.open(&path, Permission::ReadOnly).unwrap();
    assert!(readonly.truncate(7).is_err());
    assert_eq!(fs.account.usage().unwrap().managed_file_extent_bytes, 1024);
    writer.truncate(7).unwrap();
    assert_eq!(fs.account.usage().unwrap().managed_file_extent_bytes, 7);
    writer.seek(SeekFrom::Start(7)).unwrap();
    writer.write_all(b"more").unwrap();
    assert_eq!(fs.account.usage().unwrap().managed_file_extent_bytes, 11);
    drop(writer);
    drop(handle);
    drop(readonly);
    drop(fs);
    let fs = setup(dir.path());
    assert_eq!(fs.account.usage().unwrap().managed_file_extent_bytes, 11);
}
#[test]
fn reservation_denial_has_no_file_side_effects_and_maintenance_uses_protected_slack() {
    let dir = tempfile::tempdir().unwrap();
    let account = Account::inventory(dir.path(), 256 * 1024, 128 * 1024).unwrap();
    assert!(
        account
            .reserve(128 * 1024, ReservationKind::Admission)
            .is_err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    let reserve = account
        .reserve(128 * 1024, ReservationKind::Maintenance)
        .unwrap();
    assert_eq!(account.usage().unwrap().reserved_bytes, 192 * 1024);
    drop(reserve);
    assert_eq!(account.usage().unwrap().reserved_bytes, 64 * 1024);
}
#[test]
fn inventory_over_budget_preserves_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old");
    std::fs::write(&path, vec![0; 200 * 1024]).unwrap();
    assert!(Account::inventory(dir.path(), 256 * 1024, 64 * 1024).is_err());
    assert_eq!(std::fs::metadata(path).unwrap().len(), 200 * 1024);
}
