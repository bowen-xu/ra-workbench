use super::*;
use crate::agent::repository::change_sets;

fn detach(f: &crate::agent::writing_submission::tests::Fixture) {
    let db = f.store.connection().unwrap();
    db.execute(
        "update agent_conversations set archived=1 where id=?1",
        [f.conversation.to_string()],
    )
    .unwrap();
    db.execute(
        "delete from agent_conversation_bindings where conversation_id=?1",
        [f.conversation.to_string()],
    )
    .unwrap();
}

#[tokio::test]
async fn admitted_files_and_independent_undo_survive_archived_unbound_chat() {
    for removed in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let original = existing(&f, removed).await;
        detach(&f);
        let applied = service
            .apply_disk_proposal_file(original, "main.tex".into())
            .await
            .unwrap();
        assert_eq!(applied.status, ApplicationStatus::Applied);
        assert_eq!(
            service
                .find_proposal_application(
                    original.conversation,
                    original.manuscript,
                    original.proposal
                )
                .await
                .unwrap(),
            Some(applied.clone())
        );
        assert!(
            service
                .find_latest_proposal_undo(original)
                .await
                .unwrap()
                .is_none()
        );
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
        service
            .defer_proposal_undo(original, operation)
            .await
            .unwrap();
        service
            .resume_proposal_undo(original, operation, request())
            .await
            .unwrap();
        let done = service
            .apply_disk_undo_file(original, operation, "main.tex".into())
            .await
            .unwrap();
        assert_eq!(done.application.status, ApplicationStatus::Applied);
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
                .unwrap(),
            applied
        );
        std::fs::write(source(&f).join("main.tex"), b"later input").unwrap();
        assert_eq!(
            service
                .reconcile_disk_undo_file(original, operation, "main.tex".into())
                .await
                .unwrap()
                .application,
            done.application
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"later input"
        );
    }
}

#[tokio::test]
async fn detached_file_authority_keeps_scope_and_new_admission_protection() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    detach(&f);
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    assert!(
        service
            .preflight_proposal(f.conversation, original.proposal, request())
            .await
            .is_err()
    );
    assert!(
        service
            .begin_proposal_application(
                f.conversation,
                original.proposal,
                Uuid::new_v4(),
                request()
            )
            .await
            .is_err()
    );
    for wrong in [
        ApplicationScope {
            conversation: Uuid::new_v4(),
            ..original
        },
        ApplicationScope {
            manuscript: Uuid::new_v4(),
            ..original
        },
        ApplicationScope {
            proposal: Uuid::new_v4(),
            ..original
        },
        ApplicationScope {
            operation: Uuid::new_v4(),
            ..original
        },
    ] {
        assert!(
            service
                .prepare_proposal_undo(wrong, Uuid::new_v4(), request())
                .await
                .is_err()
        );
    }
    f.store
        .connection()
        .unwrap()
        .execute(
            "update manuscripts set status='archived' where id=?1",
            [f.manuscript.to_string()],
        )
        .unwrap();
    assert!(
        service
            .read_proposal_application(
                original.conversation,
                original.manuscript,
                original.proposal,
                original.operation
            )
            .await
            .is_err()
    );
    assert!(
        service
            .prepare_proposal_undo(original, Uuid::new_v4(), request())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
}

#[tokio::test]
async fn file_origin_requires_durable_admission_without_opening_new_execution() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    assert!(
        change_sets::read_in(
            &f.store.connection().unwrap(),
            f.conversation,
            f.manuscript,
            proposal
        )
        .is_ok()
    );
    assert!(
        change_sets::read_file_operation_in(
            &f.store.connection().unwrap(),
            f.conversation,
            f.manuscript,
            proposal
        )
        .is_err()
    );
    detach(&f);
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    assert!(
        service
            .preflight_proposal(f.conversation, proposal, request())
            .await
            .is_err()
    );
    assert!(
        service
            .begin_proposal_application(f.conversation, proposal, Uuid::new_v4(), request())
            .await
            .is_err()
    );
    assert!(
        service
            .prepare(&f.models, super::request(&f, Uuid::new_v4()))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
}

#[tokio::test]
async fn detached_partial_undo_checks_all_files_before_compensation() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing_with_extra(&f, false, true).await;
    for path in ["main.tex", "other.tex"] {
        service
            .apply_disk_proposal_file(original, path.into())
            .await
            .unwrap();
    }
    let undo = Uuid::new_v4();
    let request = || WritingWorkspaceRequest {
        manuscript_id: f.manuscript,
        buffers: vec![],
    };
    service
        .prepare_proposal_undo(original, undo, request())
        .await
        .unwrap();
    service
        .start_proposal_undo(original, undo, request())
        .await
        .unwrap();
    service
        .apply_disk_undo_file(original, undo, "main.tex".into())
        .await
        .unwrap();
    detach(&f);
    std::fs::write(source(&f).join("other.tex"), b"later input").unwrap();
    assert!(
        service
            .prepare_proposal_undo_rollback(original, undo, request())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert_eq!(
        std::fs::read(source(&f).join("other.tex")).unwrap(),
        b"later input"
    );
    std::fs::write(source(&f).join("other.tex"), b"extra").unwrap();
    service
        .prepare_proposal_undo_rollback(original, undo, request())
        .await
        .unwrap();
    let done = service
        .rollback_disk_undo_file(original, undo, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::RolledBack);
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
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
