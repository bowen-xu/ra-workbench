use super::*;

#[tokio::test]
async fn rollback_added_removes_a_matching_registered_index() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let scope = added(&f, b"bytes").await;
    f.store.connection().unwrap().execute_batch("create trigger fail_add before insert on manuscript_files when new.relative_path='new.bin' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .apply_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    {
        let mut db = f.store.connection().unwrap();
        db.execute_batch("drop trigger fail_add;").unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        f.store
            .register_created_writing_file_in(
                &tx,
                f.manuscript,
                "new.bin",
                &FileVersion {
                    sha256: format!("{:x}", Sha256::digest(b"bytes")),
                    byte_size: 5,
                    executable: false,
                },
            )
            .unwrap();
        tx.commit().unwrap();
    }
    let saved = service
        .rollback_added_proposal_file(scope, "new.bin".into())
        .await
        .unwrap();
    assert_eq!(saved.status, ApplicationStatus::RolledBack);
    let count: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select count(*) from manuscript_files where relative_path='new.bin'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn rollback_added_keeps_bytes_and_recovers_confirmation_without_deleting_user_edits() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let scope = added(&f, b"new bytes").await;
    f.store.connection().unwrap().execute_batch("create trigger fail_add before insert on manuscript_files when new.relative_path='new.bin' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .apply_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    let root = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source");
    let path = root.join("new.bin");
    std::fs::write(&path, b"user edit").unwrap();
    assert!(
        service
            .rollback_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"user edit");
    std::fs::write(&path, b"new bytes").unwrap();
    f.store.connection().unwrap().execute_batch("drop trigger fail_add; create trigger fail_restore before update of phase on agent_proposal_application_files when new.phase='restored' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .rollback_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    assert!(!path.exists());
    let backup = root.join(format!(
        ".agent-before-{}-{:x}.bak",
        scope.operation,
        Sha256::digest(b"new.bin")
    ));
    assert_eq!(std::fs::read(&backup).unwrap(), b"new bytes");
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_restore;")
        .unwrap();
    std::fs::write(&path, b"another file").unwrap();
    assert!(
        service
            .rollback_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"another file");
    std::fs::remove_file(&path).unwrap();
    let result = service
        .rollback_added_proposal_file(scope, "new.bin".into())
        .await
        .unwrap();
    assert_eq!(result.status, ApplicationStatus::RolledBack);
    assert_eq!(std::fs::read(&backup).unwrap(), b"new bytes");
    std::fs::write(&path, b"later file").unwrap();
    assert_eq!(
        service
            .rollback_added_proposal_file(scope, "new.bin".into())
            .await
            .unwrap(),
        result
    );
    assert_eq!(std::fs::read(path).unwrap(), b"later file");
}
