use super::*;

#[test]
fn replacement_rollback_never_overwrites_an_occupied_displacement_name() {
    let f = Fixture::new();
    let path = f.0.join("main.tex");
    std::fs::write(&path, b"original").unwrap();
    let target = DiskTarget::open(&f.0, "main.tex").unwrap();
    let op = Uuid::new_v4();
    let before = version(b"original", false);
    let after = version(b"replacement", false);
    target.replace(op, &before, b"replacement", &after).unwrap();
    let displaced = f.0.join(format!(
        ".agent-create-{op}-{:x}.rollback",
        Sha256::digest(b"main.tex")
    ));
    for bytes in [b"replacement".as_slice(), b"unrelated bytes".as_slice()] {
        std::fs::write(&displaced, bytes).unwrap();
        assert!(target.rollback_replaced(op, &before, &after).is_err());
        assert_eq!(std::fs::read(&displaced).unwrap(), bytes);
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(target.observe_preserved(op).unwrap(), Some(before.clone()));
    }
}

#[test]
fn replacement_rollback_preserves_both_versions_and_refuses_later_edits() {
    let f = Fixture::new();
    let path = f.0.join("main.tex");
    std::fs::write(&path, b"original").unwrap();
    let target = DiskTarget::open(&f.0, "main.tex").unwrap();
    let op = Uuid::new_v4();
    let before = version(b"original", false);
    let after = version(b"replacement", true);
    target.replace(op, &before, b"replacement", &after).unwrap();
    std::fs::write(&path, b"user edit").unwrap();
    assert!(target.rollback_replaced(op, &before, &after).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"user edit");
    std::fs::write(&path, b"replacement").unwrap();
    assert_eq!(
        target.rollback_replaced(op, &before, &after).unwrap(),
        before
    );
    let displaced = f.0.join(format!(
        ".agent-create-{op}-{:x}.rollback",
        Sha256::digest(b"main.tex")
    ));
    assert_eq!(std::fs::read(&displaced).unwrap(), b"replacement");
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(
        target.rollback_replaced(op, &before, &after).unwrap(),
        before
    );
    std::fs::write(&path, b"later edit").unwrap();
    assert!(target.rollback_replaced(op, &before, &after).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"later edit");
}

#[test]
fn replacement_rollback_resumes_after_displacement_or_before_publication() {
    for published in [false, true] {
        let f = Fixture::new();
        let path = f.0.join("main.tex");
        std::fs::write(&path, b"original").unwrap();
        let target = DiskTarget::open(&f.0, "main.tex").unwrap();
        let op = Uuid::new_v4();
        let before = version(b"original", false);
        let after = version(b"replacement", false);
        if published {
            target.replace(op, &before, b"replacement", &after).unwrap();
            std::fs::rename(
                &path,
                f.0.join(format!(
                    ".agent-create-{op}-{:x}.rollback",
                    Sha256::digest(b"main.tex")
                )),
            )
            .unwrap();
        } else {
            target.remove(op, &before).unwrap();
        }
        assert_eq!(
            target.rollback_replaced(op, &before, &after).unwrap(),
            before
        );
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }
}

#[test]
fn replacement_preserves_original_and_cannot_overwrite_backup_on_retry() {
    let f = Fixture::new();
    let path = f.0.join("main.tex");
    std::fs::write(&path, b"original").unwrap();
    let target = DiskTarget::open(&f.0, "main.tex").unwrap();
    let operation = Uuid::new_v4();
    let before = version(b"original", false);
    let after = version(b"replacement", true);
    assert_eq!(
        target
            .replace(operation, &before, b"replacement", &after)
            .unwrap(),
        after
    );
    assert_eq!(
        target.observe_preserved(operation).unwrap(),
        Some(before.clone())
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
    assert!(
        target
            .replace(operation, &after, b"another", &version(b"another", false))
            .is_err()
    );
    assert_eq!(target.observe_preserved(operation).unwrap(), Some(before));
    assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
}

#[test]
fn removal_can_be_restored_but_never_overwrites_a_new_user_file() {
    let f = Fixture::new();
    let path = f.0.join("main.tex");
    std::fs::write(&path, b"original").unwrap();
    let target = DiskTarget::open(&f.0, "main.tex").unwrap();
    let operation = Uuid::new_v4();
    let before = version(b"original", false);
    target.remove(operation, &before).unwrap();
    assert!(!path.exists());
    assert_eq!(
        target.observe_preserved(operation).unwrap(),
        Some(before.clone())
    );
    std::fs::write(&path, b"new user file").unwrap();
    assert!(target.restore_preserved(operation, &before).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"new user file");
    // User removes the competing file before requesting restoration again.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        target.restore_preserved(operation, &before).unwrap(),
        before
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(target.observe_preserved(operation).unwrap(), None);
}

#[test]
fn stale_sources_and_invalid_replacement_bytes_do_not_move_original() {
    let f = Fixture::new();
    let path = f.0.join("main.tex");
    std::fs::write(&path, b"user edit").unwrap();
    let target = DiskTarget::open(&f.0, "main.tex").unwrap();
    let operation = Uuid::new_v4();
    assert!(target.remove(operation, &version(b"old", false)).is_err());
    assert!(
        target
            .replace(
                operation,
                &version(b"user edit", false),
                b"wrong",
                &version(b"expected", false)
            )
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), b"user edit");
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
}

#[test]
fn failed_publication_keeps_original_and_does_not_consume_old_staging_file() {
    let f = Fixture::new();
    let path = f.0.join("main.tex");
    std::fs::write(&path, b"original").unwrap();
    let target = DiskTarget::open(&f.0, "main.tex").unwrap();
    let operation = Uuid::new_v4();
    let before = version(b"original", false);
    let staging = f.0.join(target.temporary_name(operation));
    std::fs::write(&staging, b"interrupted attempt").unwrap();
    assert!(matches!(
        target.replace(operation, &before, b"new", &version(b"new", false)),
        Err(DiskError::RecoveryRequired)
    ));
    assert!(!path.exists());
    assert_eq!(
        target.observe_preserved(operation).unwrap(),
        Some(before.clone())
    );
    assert_eq!(std::fs::read(&staging).unwrap(), b"interrupted attempt");
    assert_eq!(
        target.restore_preserved(operation, &before).unwrap(),
        before
    );
    assert_eq!(std::fs::read(path).unwrap(), b"original");
    assert_eq!(std::fs::read(staging).unwrap(), b"interrupted attempt");
}

#[test]
fn changed_backup_cannot_be_restored_as_the_frozen_original() {
    let f = Fixture::new();
    let path = f.0.join("main.tex");
    std::fs::write(&path, b"original").unwrap();
    let target = DiskTarget::open(&f.0, "main.tex").unwrap();
    let operation = Uuid::new_v4();
    let before = version(b"original", false);
    target.remove(operation, &before).unwrap();
    let backup = std::fs::read_dir(&f.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(&backup, b"external change").unwrap();
    assert!(matches!(
        target.restore_preserved(operation, &before),
        Err(DiskError::Conflict)
    ));
    assert!(!path.exists());
    assert_eq!(std::fs::read(backup).unwrap(), b"external change");
}
