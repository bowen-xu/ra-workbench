use super::*;
#[test]
fn extracts_all_present_versions_including_binary_large_empty_and_deleted_files() {
    let temp = tempfile::tempdir().unwrap();
    let lease = store(temp.path()).create(manifest(), input()).unwrap();
    let work = lease.working_directory().unwrap();
    fs::remove_file(work.join("sections/main.tex")).unwrap();
    fs::write(work.join("empty"), []).unwrap();
    let binary = vec![0xff; 1024 * 1024 + 1];
    fs::write(work.join("binary"), &binary).unwrap();
    let changes = lease.changes().unwrap();
    let mut captured = vec![];
    lease
        .visit_change_bytes(&changes, |change, side, bytes| {
            captured.push((change.path.clone(), side, bytes.to_vec()));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        captured,
        vec![
            ("binary".into(), ChangeSide::After, binary),
            ("empty".into(), ChangeSide::After, vec![]),
            (
                "sections/main.tex".into(),
                ChangeSide::Before,
                b"user buffer".to_vec()
            ),
        ]
    );
    let mut touched = false;
    assert_eq!(
        lease.visit_change_bytes(&changes[..1], |_, _, _| {
            touched = true;
            Ok(())
        }),
        Err(LeaseError::Conflict)
    );
    assert!(!touched);
}
#[test]
fn rejects_stale_source_metadata_and_changes_made_during_capture() {
    let temp = tempfile::tempdir().unwrap();
    let lease = store(temp.path()).create(manifest(), input()).unwrap();
    let work = lease.working_directory().unwrap();
    fs::write(work.join("sections/main.tex"), b"new").unwrap();
    let changes = lease.changes().unwrap();
    let mut forged = changes.clone();
    forged[0].source.as_mut().unwrap().revision = "other".into();
    assert_eq!(
        lease.visit_change_bytes(&forged, |_, _, _| panic!("must reject before output")),
        Err(LeaseError::Conflict)
    );
    let result = lease.visit_change_bytes(&changes, |_, side, _| {
        if side == ChangeSide::After {
            fs::write(work.join("unexpected"), b"new path").unwrap();
        }
        Ok(())
    });
    assert_eq!(result, Err(LeaseError::Conflict));
    fs::remove_file(work.join("unexpected")).unwrap();
    assert_eq!(
        lease.visit_change_bytes(&changes, |_, _, _| Err(LeaseError::Unavailable)),
        Err(LeaseError::Unavailable)
    );
    assert!(lease.visit_change_bytes(&changes, |_, _, _| Ok(())).is_ok());
}
#[test]
fn validates_links_modes_and_execution_quiescence_before_capture() {
    let temp = tempfile::tempdir().unwrap();
    let mut lease = store(temp.path()).create(manifest(), input()).unwrap();
    let work = lease.working_directory().unwrap();
    fs::write(work.join("new"), b"body").unwrap();
    let changes = lease.changes().unwrap();
    fs::hard_link(work.join("new"), temp.path().join("alias")).unwrap();
    assert!(
        lease
            .visit_change_bytes(&changes, |_, _, _| panic!("hardlink"))
            .is_err()
    );
    fs::remove_file(temp.path().join("alias")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(work.join("new"), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        lease.visit_change_bytes(&changes, |_, _, _| panic!("mode change")),
        Err(LeaseError::Conflict)
    );
    let execution = lease.begin_execution().unwrap();
    assert_eq!(
        lease.visit_change_bytes(&changes, |_, _, _| panic!("active writer")),
        Err(LeaseError::Busy)
    );
    drop(execution);
}
