use super::*;

#[tokio::test]
async fn disk_undo_restores_all_change_kinds_and_keeps_original_receipts() {
    for kind in ["add", "remove", "replace"] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let original = if kind == "add" {
            super::super::apply_added::added(&f, &[0xff, 0, 42]).await
        } else {
            existing(&f, kind == "remove").await
        };
        let path = if kind == "add" { "new.bin" } else { "main.tex" };
        let applied = service
            .apply_disk_proposal_file(original, path.into())
            .await
            .unwrap();
        let operation = Uuid::new_v4();
        let request = || WritingWorkspaceRequest {
            manuscript_id: f.manuscript,
            buffers: vec![],
        };
        service
            .prepare_proposal_undo(original, operation, request())
            .await
            .unwrap();
        assert!(
            service
                .apply_disk_undo_file(original, operation, path.into())
                .await
                .is_err()
        );
        service
            .start_proposal_undo(original, operation, request())
            .await
            .unwrap();
        let done = service
            .apply_disk_undo_file(original, operation, path.into())
            .await
            .unwrap();
        assert_eq!(done.application.status, ApplicationStatus::Applied);
        assert_eq!(
            done.application.files[0].phase,
            ApplicationPhase::Acknowledged
        );
        if kind == "add" {
            assert!(!source(&f).join(path).exists());
        } else {
            assert_eq!(
                std::fs::read(source(&f).join(path)).unwrap(),
                b"Frozen manuscript"
            );
        }
        let count: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_files where manuscript_id=?1 and relative_path=?2",
                rusqlite::params![f.manuscript.to_string(), path],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, if kind == "add" { 0 } else { 1 });
        assert_eq!(
            service
                .read_proposal_application(
                    original.conversation,
                    original.manuscript,
                    original.proposal,
                    original.operation
                )
                .await
                .unwrap(),
            applied
        );
        std::fs::write(source(&f).join(path), b"later user input").unwrap();
        assert_eq!(
            service
                .apply_disk_undo_file(original, operation, path.into())
                .await
                .unwrap()
                .application,
            done.application
        );
        assert_eq!(
            service
                .reconcile_disk_undo_file(original, operation, path.into())
                .await
                .unwrap()
                .application,
            done.application
        );
        assert_eq!(
            std::fs::read(source(&f).join(path)).unwrap(),
            b"later user input"
        );
    }
}

#[tokio::test]
async fn failed_undo_confirmation_retains_intent_and_reconciles_without_replay() {
    for removed in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let original = existing(&f, removed).await;
        service
            .apply_disk_proposal_file(original, "main.tex".into())
            .await
            .unwrap();
        let operation = Uuid::new_v4();
        let request = || WritingWorkspaceRequest {
            manuscript_id: f.manuscript,
            buffers: vec![],
        };
        service
            .prepare_proposal_undo(original, operation, request())
            .await
            .unwrap();
        service
            .start_proposal_undo(original, operation, request())
            .await
            .unwrap();
        assert!(
            service
                .reconcile_disk_undo_file(original, operation, "main.tex".into())
                .await
                .is_err()
        );
        f.store.connection().unwrap().execute_batch("create trigger fail_undo_finish before update of status on agent_change_sets when new.status='undone' begin select raise(abort,'fixture'); end;").unwrap();
        assert!(matches!(
            service
                .apply_disk_undo_file(original, operation, "main.tex".into())
                .await,
            Err(Error::RecoveryRequired)
        ));
        let saved = service
            .read_proposal_undo(original, operation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.application.status,
            ApplicationStatus::RecoveryRequired
        );
        assert_eq!(
            saved.application.files[0].phase,
            ApplicationPhase::Dispatched
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"Frozen manuscript"
        );
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_undo_finish")
            .unwrap();
        std::fs::write(source(&f).join("main.tex"), b"user correction").unwrap();
        assert_eq!(
            service
                .apply_disk_undo_file(original, operation, "main.tex".into())
                .await
                .unwrap()
                .application,
            saved.application
        );
        assert!(
            service
                .reconcile_disk_undo_file(original, operation, "main.tex".into())
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"user correction"
        );
        std::fs::write(source(&f).join("main.tex"), b"Frozen manuscript").unwrap();
        let done = service
            .reconcile_disk_undo_file(original, operation, "main.tex".into())
            .await
            .unwrap();
        assert_eq!(done.application.status, ApplicationStatus::Applied);
        let hash: String = f.store.connection().unwrap().query_row("select sha256 from manuscript_files where manuscript_id=?1 and relative_path='main.tex'", [f.manuscript.to_string()], |r|r.get(0)).unwrap();
        assert_eq!(hash, f.hash);
    }
}
