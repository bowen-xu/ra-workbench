use super::*;
use crate::agent::{repository::change_sets::ChangeSetStatus, workspace::ReviewContent};

#[tokio::test]
async fn rejection_is_idempotent_and_keeps_frozen_bytes_references_and_sources() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let id = service
        .publish_workspace_changes(review.binding.clone(), review.changes.clone())
        .await
        .unwrap();
    for _ in 0..2 {
        let rejected = service
            .reject_proposal(f.conversation, f.manuscript, id)
            .await
            .unwrap();
        assert_eq!(rejected.status, ChangeSetStatus::Rejected);
        assert_eq!(rejected.changes, review.changes);
        assert_eq!(rejected.binding, review.binding);
    }
    // Republishing the same bytes cannot resurrect a rejected proposal.
    assert_eq!(
        service
            .publish_workspace_changes(review.binding, review.changes)
            .await
            .unwrap(),
        id
    );
    assert_eq!(
        service
            .review_proposal(f.conversation, f.manuscript, id)
            .await
            .unwrap()
            .status,
        ChangeSetStatus::Rejected
    );
    let file = service
        .review_proposal_file(f.conversation, f.manuscript, id, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(
        file.after,
        Some(ReviewContent::Text("Revised manuscript".into()))
    );
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from content_object_gc_candidates",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_change_set_files where change_set_id=?1",
            [id.to_string()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"Frozen manuscript"
    );
}

#[tokio::test]
async fn rejection_checks_scope_and_cannot_overwrite_application_or_recovery_states() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let id = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    assert!(
        service
            .reject_proposal(Uuid::new_v4(), f.manuscript, id)
            .await
            .is_err()
    );
    assert!(
        service
            .reject_proposal(f.conversation, Uuid::new_v4(), id)
            .await
            .is_err()
    );
    assert!(
        service
            .reject_proposal(f.conversation, f.manuscript, Uuid::nil())
            .await
            .is_err()
    );
    for state in [
        "applying",
        "applied",
        "conflict",
        "recovery_required",
        "undone",
    ] {
        f.store
            .connection()
            .unwrap()
            .execute(
                "update agent_change_sets set status=?1 where id=?2",
                params![state, id.to_string()],
            )
            .unwrap();
        assert!(
            service
                .reject_proposal(f.conversation, f.manuscript, id)
                .await
                .is_err()
        );
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row(
                    "select status from agent_change_sets where id=?1",
                    [id.to_string()],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            state
        );
    }
    f.store.connection().unwrap().execute_batch("update agent_change_sets set status='pending'; delete from agent_conversation_bindings;").unwrap();
    assert!(
        service
            .reject_proposal(f.conversation, f.manuscript, id)
            .await
            .is_err()
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select status from agent_change_sets where id=?1",
                [id.to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "pending"
    );
}

#[tokio::test]
async fn rejection_storage_failure_preserves_pending_state_and_allows_retry() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let id = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    f.store.connection().unwrap().execute_batch("create trigger fail_rejection before update of status on agent_change_sets begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .reject_proposal(f.conversation, f.manuscript, id)
            .await
            .is_err()
    );
    assert_eq!(
        service
            .review_proposal(f.conversation, f.manuscript, id)
            .await
            .unwrap()
            .status,
        ChangeSetStatus::Pending
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_rejection")
        .unwrap();
    assert_eq!(
        service
            .reject_proposal(f.conversation, f.manuscript, id)
            .await
            .unwrap()
            .status,
        ChangeSetStatus::Rejected
    );
}
