use super::*;

#[tokio::test]
async fn unused_undo_can_be_cancelled_after_source_changes_and_reprepared() {
    for stage in ["prepared", "applying", "recovery"] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let original = existing(&f, false).await;
        let applied = service
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
        if stage != "prepared" {
            service
                .start_proposal_undo(original, operation, request())
                .await
                .unwrap();
        }
        if stage == "recovery" {
            service
                .defer_proposal_undo(original, operation)
                .await
                .unwrap();
        }
        std::fs::write(source(&f).join("main.tex"), b"later input").unwrap();
        let cancelled = service
            .cancel_pending_proposal_undo(original, operation)
            .await
            .unwrap();
        assert_eq!(cancelled.application.status, ApplicationStatus::RolledBack);
        assert_eq!(
            cancelled.application.files[0].phase,
            ApplicationPhase::Pending
        );
        assert_eq!(
            service
                .cancel_pending_proposal_undo(original, operation)
                .await
                .unwrap()
                .application,
            cancelled.application
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"later input"
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
                .unwrap(),
            applied
        );
        assert!(
            service
                .apply_disk_undo_file(original, operation, "main.tex".into())
                .await
                .is_err()
        );
        assert!(
            service
                .prepare_proposal_undo(original, Uuid::new_v4(), request())
                .await
                .is_err()
        );
        std::fs::write(source(&f).join("main.tex"), b"replacement").unwrap();
        let next = Uuid::new_v4();
        service
            .prepare_proposal_undo(original, next, request())
            .await
            .unwrap();
        service
            .start_proposal_undo(original, next, request())
            .await
            .unwrap();
        service
            .apply_disk_undo_file(original, next, "main.tex".into())
            .await
            .unwrap();
        assert!(
            service
                .cancel_pending_proposal_undo(original, next)
                .await
                .is_err()
        );
        // The old cancellation remains historical even after a later undo.
        assert_eq!(
            service
                .cancel_pending_proposal_undo(original, operation)
                .await
                .unwrap()
                .application,
            cancelled.application
        );
    }
}

#[tokio::test]
async fn cancellation_refuses_a_dispatched_undo_and_preserves_recovery() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
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
    f.store.connection().unwrap().execute_batch("create trigger fail_undo_ack before update of status on agent_change_sets when new.status='undone' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .apply_disk_undo_file(original, operation, "main.tex".into())
            .await
            .is_err()
    );
    let before = service
        .read_proposal_undo(original, operation)
        .await
        .unwrap()
        .unwrap();
    assert!(
        service
            .cancel_pending_proposal_undo(original, operation)
            .await
            .is_err()
    );
    assert_eq!(
        service
            .read_proposal_undo(original, operation)
            .await
            .unwrap()
            .unwrap()
            .application,
        before.application
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
}
