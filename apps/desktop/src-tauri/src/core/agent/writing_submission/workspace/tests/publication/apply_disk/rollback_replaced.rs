use super::*;

#[tokio::test]
async fn replacement_rollback_recovers_old_or_new_index_and_lost_receipt() {
    for new_index in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let path = source(&f).join("main.tex");
        let original = std::fs::read(&path).unwrap();
        let scope = existing(&f, false).await;
        f.store.connection().unwrap().execute_batch("create trigger fail_update before update on manuscript_files begin select raise(abort,'fixture'); end;").unwrap();
        assert!(
            service
                .apply_disk_proposal_file(scope, "main.tex".into())
                .await
                .is_err()
        );
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_update;")
            .unwrap();
        if new_index {
            f.store.connection().unwrap().execute("update manuscript_files set byte_size=11,sha256=?1 where relative_path='main.tex'",[format!("{:x}",Sha256::digest(b"replacement"))]).unwrap();
        }
        std::fs::write(&path, b"user change").unwrap();
        assert!(
            service
                .rollback_replaced_proposal_file(scope, "main.tex".into())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"user change");
        std::fs::write(&path, b"replacement").unwrap();
        f.store.connection().unwrap().execute_batch("create trigger fail_restore before update of phase on agent_proposal_application_files when new.phase='restored' begin select raise(abort,'fixture'); end;").unwrap();
        assert!(
            service
                .rollback_replaced_proposal_file(scope, "main.tex".into())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_restore;")
            .unwrap();
        let saved = service
            .rollback_replaced_proposal_file(scope, "main.tex".into())
            .await
            .unwrap();
        assert_eq!(saved.status, ApplicationStatus::RolledBack);
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
        let displaced = source(&f).join(format!(
            ".agent-create-{}-{:x}.rollback",
            scope.operation,
            Sha256::digest(b"main.tex")
        ));
        assert_eq!(std::fs::read(displaced).unwrap(), b"replacement");
        std::fs::write(&path, b"later edit").unwrap();
        assert_eq!(
            service
                .rollback_replaced_proposal_file(scope, "main.tex".into())
                .await
                .unwrap(),
            saved
        );
        assert_eq!(std::fs::read(path).unwrap(), b"later edit");
    }
}
