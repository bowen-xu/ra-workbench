use super::*;

#[test]
fn completion_survives_reopen_and_fences_same_identity_recreation() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = manifest();
    {
        let copies = store(temp.path());
        drop(copies.create(manifest.clone(), input()).unwrap());
        copies.discard(&manifest.identity).unwrap();
    }
    let copies = store(temp.path());
    let receipt = temp.path().join(super::super::super::discard::receipt_name(
        &manifest.identity,
    ));
    assert!(receipt.is_file());
    let saved = fs::read(&receipt).unwrap();
    copies.discard(&manifest.identity).unwrap();
    assert_eq!(fs::read(&receipt).unwrap(), saved);
    assert!(matches!(
        copies.create(manifest.clone(), input()),
        Err(LeaseError::Conflict)
    ));
    assert!(matches!(
        copies.restore(&manifest.identity),
        Err(LeaseError::Incomplete)
    ));
    let mut wrong = manifest.identity.clone();
    wrong.turn_id = Uuid::new_v4();
    assert_eq!(copies.discard(&wrong), Err(LeaseError::Conflict));
    // A new explicit lease identity remains independent.
    let mut next = manifest.clone();
    next.identity.lease_id = Uuid::new_v4();
    drop(copies.create(next.clone(), input()).unwrap());
    copies.discard(&next.identity).unwrap();
}

#[test]
fn completed_receipt_never_authorizes_deleting_replacement_even_with_matching_inode() {
    let temp = tempfile::tempdir().unwrap();
    let copies = store(temp.path());
    let manifest = manifest();
    drop(copies.create(manifest.clone(), input()).unwrap());
    copies.discard(&manifest.identity).unwrap();
    let path = temp.path().join(manifest.identity.lease_id.to_string());
    fs::create_dir(&path).unwrap();
    fs::write(path.join("keep"), b"replacement").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    // Simulate recycled identity: completion itself must fence deletion, rather
    // than trusting the original device/inode pair forever.
    fs::write(temp.path().join(super::super::super::discard::receipt_name(&manifest.identity)),
        serde_json::to_vec(&serde_json::json!({"identity":manifest.identity,"device":metadata.dev(),"inode":metadata.ino()})).unwrap()).unwrap();
    assert_eq!(
        copies.discard(&manifest.identity),
        Err(LeaseError::Conflict)
    );
    assert_eq!(fs::read(path.join("keep")).unwrap(), b"replacement");
}

#[test]
fn interrupted_removal_without_child_reaches_durable_completion() {
    let temp = tempfile::tempdir().unwrap();
    let copies = store(temp.path());
    let manifest = manifest();
    drop(copies.create(manifest.clone(), input()).unwrap());
    let path = temp.path().join(manifest.identity.lease_id.to_string());
    let metadata = fs::metadata(&path).unwrap();
    let journal = temp.path().join(super::super::super::discard::journal_name(
        &manifest.identity,
    ));
    fs::write(&journal, serde_json::to_vec(&serde_json::json!({"identity":manifest.identity,"device":metadata.dev(),"inode":metadata.ino()})).unwrap()).unwrap();
    fs::remove_dir_all(&path).unwrap();
    copies.discard(&manifest.identity).unwrap();
    assert!(!journal.exists());
    copies.discard(&manifest.identity).unwrap();
}

#[test]
fn corrupt_or_oversized_receipt_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let copies = store(temp.path());
    let manifest = manifest();
    let receipt = temp.path().join(super::super::super::discard::receipt_name(
        &manifest.identity,
    ));
    fs::write(&receipt, b"{}").unwrap();
    assert_eq!(copies.discard(&manifest.identity), Err(LeaseError::Invalid));
    fs::write(&receipt, vec![b'x'; 8193]).unwrap();
    assert_eq!(copies.discard(&manifest.identity), Err(LeaseError::Budget));
}

#[test]
fn reviewed_discard_journal_is_published_before_ownership_is_released() {
    let temp = tempfile::tempdir().unwrap();
    let copies = store(temp.path());
    let manifest = manifest();
    let lease = copies.create(manifest.clone(), input()).unwrap();
    assert_eq!(copies.discard_state(&manifest.identity).unwrap(), None);
    lease.prepare_discard(&[]).unwrap();
    assert_eq!(
        copies.discard_state(&manifest.identity).unwrap(),
        Some(super::super::super::DiscardState::Pending)
    );
    assert!(matches!(
        copies.restore(&manifest.identity),
        Err(LeaseError::Incomplete)
    ));
    copies.discard(&manifest.identity).unwrap();
    assert_eq!(
        copies.discard_state(&manifest.identity).unwrap(),
        Some(super::super::super::DiscardState::Complete)
    );
}

#[test]
fn stale_review_never_publishes_a_discard_journal() {
    let temp = tempfile::tempdir().unwrap();
    let copies = store(temp.path());
    let manifest = manifest();
    let lease = copies.create(manifest.clone(), input()).unwrap();
    fs::write(
        lease.working_directory().unwrap().join("later.tex"),
        b"keep",
    )
    .unwrap();
    assert_eq!(lease.prepare_discard(&[]), Err(LeaseError::Conflict));
    assert_eq!(copies.discard_state(&manifest.identity).unwrap(), None);
    let restored = copies.restore(&manifest.identity).unwrap();
    assert_eq!(
        fs::read(restored.working_directory().unwrap().join("later.tex")).unwrap(),
        b"keep"
    );
}
