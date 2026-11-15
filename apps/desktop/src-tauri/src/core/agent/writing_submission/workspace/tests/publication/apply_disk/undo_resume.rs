use super::*;

#[tokio::test]
async fn undo_resume_checks_pending_and_completed_files_without_replaying() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing_with_extra(&f, false, true).await;
    for path in ["main.tex", "other.tex"] {
        service
            .apply_disk_proposal_file(original, path.into())
            .await
            .unwrap();
    }
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    let operation = Uuid::new_v4();
    service
        .prepare_proposal_undo(original, operation, request())
        .await
        .unwrap();
    service
        .start_proposal_undo(original, operation, request())
        .await
        .unwrap();
    service
        .apply_disk_undo_file(original, operation, "main.tex".into())
        .await
        .unwrap();
    service
        .defer_proposal_undo(original, operation)
        .await
        .unwrap();
    for (path, restored) in [
        ("main.tex", b"Frozen manuscript".as_slice()),
        ("other.tex", b"extra".as_slice()),
    ] {
        std::fs::write(source(&f).join(path), b"later input").unwrap();
        assert!(
            service
                .resume_proposal_undo(original, operation, request())
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(source(&f).join(path)).unwrap(),
            b"later input"
        );
        assert_eq!(
            service
                .read_proposal_undo(original, operation)
                .await
                .unwrap()
                .unwrap()
                .application
                .status,
            ApplicationStatus::RecoveryRequired
        );
        std::fs::write(source(&f).join(path), restored).unwrap();
    }
    let object_path: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select relative_path from content_objects where sha256=?1",
            [&f.hash],
            |r| r.get(0),
        )
        .unwrap();
    let object_path = f.store.content_directory().join(object_path);
    let bytes = std::fs::read(&object_path).unwrap();
    std::fs::write(&object_path, b"corrupted").unwrap();
    assert!(
        service
            .resume_proposal_undo(original, operation, request())
            .await
            .is_err()
    );
    std::fs::write(&object_path, bytes).unwrap();
    let resumed = service
        .resume_proposal_undo(original, operation, request())
        .await
        .unwrap();
    assert_eq!(resumed.application.status, ApplicationStatus::Applying);
    assert_eq!(
        resumed
            .application
            .files
            .iter()
            .find(|f| f.path == "main.tex")
            .unwrap()
            .phase,
        ApplicationPhase::Acknowledged
    );
    assert_eq!(
        service
            .resume_proposal_undo(original, operation, request())
            .await
            .unwrap()
            .application,
        resumed.application
    );
    let done = service
        .apply_disk_undo_file(original, operation, "other.tex".into())
        .await
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::Applied);
    assert!(!source(&f).join("other.tex").exists());
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert_eq!(
        service
            .read_proposal_application(
                original.conversation,
                original.manuscript,
                original.proposal,
                original.operation
            )
            .await
            .unwrap()
            .status,
        ApplicationStatus::Applied
    );
}

#[tokio::test]
async fn undo_resume_refuses_unreconciled_dispatch() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    let operation = Uuid::new_v4();
    service
        .prepare_proposal_undo(original, operation, request())
        .await
        .unwrap();
    service
        .start_proposal_undo(original, operation, request())
        .await
        .unwrap();
    {
        let mut db = f.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        crate::agent::repository::proposal_undos::dispatch_in(&tx, original, operation, "main.tex")
            .unwrap();
        tx.commit().unwrap();
    }
    service
        .defer_proposal_undo(original, operation)
        .await
        .unwrap();
    assert!(
        service
            .resume_proposal_undo(original, operation, request())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
    assert_eq!(
        service
            .read_proposal_undo(original, operation)
            .await
            .unwrap()
            .unwrap()
            .application
            .files[0]
            .phase,
        ApplicationPhase::Dispatched
    );
}
