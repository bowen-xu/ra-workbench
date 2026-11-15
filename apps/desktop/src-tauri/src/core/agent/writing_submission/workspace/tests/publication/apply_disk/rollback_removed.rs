use super::*;

#[tokio::test]
async fn rollback_recreates_index_after_a_confirmed_deletion_in_an_unfinished_apply() {
    use crate::agent::{repository::proposal_applications as applications, workspace::DiskTarget};
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let scope = existing(&f, true).await;
    let target = DiskTarget::open(&source(&f).canonicalize().unwrap(), "main.tex").unwrap();
    let before = target.observe().unwrap().unwrap();
    {
        let mut db = f.store.connection().unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        applications::dispatch_in(&tx, scope, "main.tex").unwrap();
        tx.commit().unwrap();
    }
    target.remove(scope.operation, &before).unwrap();
    {
        let mut db = f.store.connection().unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "delete from manuscript_files where manuscript_id=?1 and relative_path='main.tex'",
            [f.manuscript.to_string()],
        )
        .unwrap();
        applications::acknowledge_in(
            &tx,
            scope,
            &applications::FileReceipt {
                path: "main.tex".into(),
                after: None,
                editor: None,
            },
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let result = service
        .rollback_removed_proposal_file(scope, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(result.status, ApplicationStatus::RolledBack);
    assert_eq!(target.observe().unwrap(), Some(before.clone()));
    let hash: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select sha256 from manuscript_files where relative_path='main.tex'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hash, before.sha256);
}

#[tokio::test]
async fn deletion_rollback_restores_bytes_and_recovers_a_lost_confirmation() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let path = source(&f).join("main.tex");
    let original = std::fs::read(&path).unwrap();
    let scope = existing(&f, true).await;
    assert!(
        service
            .rollback_removed_proposal_file(scope, "main.tex".into())
            .await
            .is_err()
    );
    f.store.connection().unwrap().execute_batch("create trigger fail_delete before delete on manuscript_files begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .apply_disk_proposal_file(scope, "main.tex".into())
            .await
            .is_err()
    );
    assert!(!path.exists());
    std::fs::write(&path, b"user new file").unwrap();
    assert!(
        service
            .rollback_removed_proposal_file(scope, "main.tex".into())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"user new file");
    std::fs::remove_file(&path).unwrap();
    f.store.connection().unwrap().execute_batch("drop trigger fail_delete; create trigger fail_restore before update of phase on agent_proposal_application_files when new.phase='restored' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .rollback_removed_proposal_file(scope, "main.tex".into())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::write(&path, b"new edit after restoration").unwrap();
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_restore;")
        .unwrap();
    assert!(
        service
            .rollback_removed_proposal_file(scope, "main.tex".into())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"new edit after restoration");
    std::fs::write(&path, &original).unwrap();
    let result = service
        .rollback_removed_proposal_file(scope, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(result.status, ApplicationStatus::RolledBack);
    assert_eq!(result.files[0].phase, ApplicationPhase::Restored);
    let hash: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select sha256 from manuscript_files where relative_path='main.tex'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hash, format!("{:x}", Sha256::digest(&original)));
    std::fs::write(&path, b"later edit").unwrap();
    assert_eq!(
        service
            .rollback_removed_proposal_file(scope, "main.tex".into())
            .await
            .unwrap(),
        result
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"later edit");
}
