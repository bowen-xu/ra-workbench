use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    BindingKind, ConversationBinding, ConversationDeletionRequest, ConversationDeletionStore,
    ConversationId,
};

pub(in super::super) fn full_fixture() -> crate::agent::writing_submission::tests::Fixture {
    fixture_with_objects()
}

pub(in super::super) async fn delete_chat(f: &crate::agent::writing_submission::tests::Fixture) {
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    let catalog = SqliteConversationCatalog::from_app_store(&f.store);
    let (title, count) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select title,durable_count from agent_conversations where id=?1",
            [f.conversation.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    catalog
        .delete(ConversationDeletionRequest {
            request_id: Uuid::new_v4(),
            conversation_id: ConversationId::from_uuid(f.conversation),
            binding: Some(ConversationBinding {
                kind: BindingKind::Manuscript,
                resource_id: f.manuscript.to_string(),
            }),
            expected_title: title,
            expected_durable_count: count,
            force: true,
        })
        .await
        .unwrap();
    let db = f.store.connection().unwrap();
    for table in [
        "agent_conversations",
        "agent_submissions",
        "agent_execution_workspaces",
    ] {
        assert_eq!(
            db.query_row(&format!("select count(*) from {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn admitted_delivery_provenance_remains_readable_after_archive_and_permanent_deletion() {
    use crate::agent::workspace::ReviewContent;
    let f = full_fixture();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    for deleted in [false, true] {
        if deleted {
            delete_chat(&f).await;
        }
        let review = service
            .review_proposal(f.conversation, f.manuscript, original.proposal)
            .await
            .unwrap();
        assert_eq!(review.id, original.proposal);
        assert_eq!(review.binding.conversation_id, f.conversation);
        let file = service
            .review_proposal_file(
                f.conversation,
                f.manuscript,
                original.proposal,
                "main.tex".into(),
            )
            .await
            .unwrap();
        assert_eq!(
            file.before,
            Some(ReviewContent::Text("Frozen manuscript".into()))
        );
        assert_eq!(file.after, Some(ReviewContent::Text("replacement".into())));
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"Frozen manuscript"
        );
        assert!(
            service
                .review_proposal(Uuid::new_v4(), f.manuscript, original.proposal)
                .await
                .is_err()
        );
        assert!(
            service
                .review_proposal(f.conversation, Uuid::new_v4(), original.proposal)
                .await
                .is_err()
        );
    }
    assert!(
        service
            .begin_proposal_application(
                f.conversation,
                original.proposal,
                Uuid::new_v4(),
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![]
                }
            )
            .await
            .is_err()
    );
    // Corruption is injected only after proving the production trigger rejects
    // it; a read fallback must still validate frozen provenance independently.
    assert!(
        f.store
            .connection()
            .unwrap()
            .execute(
                "update agent_change_sets set baseline_json='{}' where id=?1",
                [original.proposal.to_string()],
            )
            .is_err()
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger agent_change_set_baseline_immutable")
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_change_sets set baseline_json='{}' where id=?1",
            [original.proposal.to_string()],
        )
        .unwrap();
    assert!(
        service
            .review_proposal(f.conversation, f.manuscript, original.proposal)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unadmitted_proposal_does_not_acquire_file_owned_review_through_archive() {
    let f = full_fixture();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.conversation.to_string()],
        )
        .unwrap();
    assert!(
        service
            .review_proposal(f.conversation, f.manuscript, proposal)
            .await
            .is_err()
    );
    assert!(
        service
            .review_proposal_file(f.conversation, f.manuscript, proposal, "main.tex".into())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn partial_file_undo_survives_permanent_chat_deletion_and_service_rebuild() {
    let f = full_fixture();
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
    service.defer_proposal_undo(original, undo).await.unwrap();
    delete_chat(&f).await;
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert_eq!(
        std::fs::read(source(&f).join("other.tex")).unwrap(),
        b"extra"
    );
    drop(service);
    let reopened_path = f.root.path().join("deleted-chat.sqlite");
    f.store
        .connection()
        .unwrap()
        .execute("vacuum into ?1", [reopened_path.to_str().unwrap()])
        .unwrap();
    let db = Connection::open(reopened_path).unwrap();
    db.execute_batch("pragma foreign_keys=on").unwrap();
    let reopened_store = AppStore::for_test(db, f.store.content_directory().to_path_buf());
    let service = WritingSubmissionService::from_app_store(&reopened_store);
    assert_eq!(
        service
            .find_latest_proposal_undo(original)
            .await
            .unwrap()
            .unwrap()
            .application
            .operation_id,
        undo
    );
    std::fs::write(source(&f).join("other.tex"), b"later input").unwrap();
    assert!(
        service
            .resume_proposal_undo(original, undo, request())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(source(&f).join("other.tex")).unwrap(),
        b"later input"
    );
    std::fs::write(source(&f).join("other.tex"), b"extra").unwrap();
    service
        .resume_proposal_undo(original, undo, request())
        .await
        .unwrap();
    let done = service
        .apply_disk_undo_file(original, undo, "other.tex".into())
        .await
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::Applied);
    assert!(!source(&f).join("other.tex").exists());
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert!(
        service
            .begin_proposal_application(
                original.conversation,
                original.proposal,
                Uuid::new_v4(),
                request()
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn admitted_pending_file_application_continues_after_deleted_origin() {
    let f = full_fixture();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    delete_chat(&f).await;
    let applied = service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(applied.status, ApplicationStatus::Applied);
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"replacement"
    );
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
    let done = service
        .apply_disk_undo_file(original, undo, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(done.application.status, ApplicationStatus::Applied);
    assert_eq!(
        std::fs::read(source(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
}
