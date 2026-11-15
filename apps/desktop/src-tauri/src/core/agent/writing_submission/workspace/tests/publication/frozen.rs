use super::*;
use crate::agent::workspace::{ReviewContent, WorkspaceStore};

#[tokio::test]
async fn confirmed_history_reopens_from_metadata_without_execution_binding_or_new_content() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let proposal = service
        .publish_workspace_changes(review.binding.clone(), review.changes)
        .await
        .unwrap();
    let database_path = f.root.path().join("reopened-history.sqlite3");
    let mut snapshot = rusqlite::Connection::open(&database_path).unwrap();
    let original_count: i64;
    {
        let db = f.store.connection().unwrap();
        db.execute(
            "update agent_change_sets set status='applied' where id=?1",
            [proposal.to_string()],
        )
        .unwrap();
        db.execute(
            "delete from agent_execution_workspaces where conversation_id=?1 and request_id=?2",
            rusqlite::params![
                f.conversation.to_string(),
                review.binding.request_id.to_string()
            ],
        )
        .unwrap();
        original_count = db
            .query_row("select count(*) from content_objects", [], |row| row.get(0))
            .unwrap();
        rusqlite::backup::Backup::new(&db, &mut snapshot)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
    }
    drop(snapshot);
    let reopened = rusqlite::Connection::open(&database_path).unwrap();
    let page = crate::agent::repository::execution_workspaces::history::list(
        &reopened,
        f.conversation,
        f.manuscript,
        None,
        1,
    )
    .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].request_id, review.binding.request_id);
    assert_eq!(page.entries[0].published_proposal_id, Some(proposal));
    assert_eq!(
        reopened
            .query_row("select count(*) from content_objects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        original_count
    );
    assert!(
        crate::agent::repository::execution_workspaces::history::list(
            &reopened,
            Uuid::new_v4(),
            f.manuscript,
            None,
            1
        )
        .unwrap()
        .entries
        .is_empty()
    );
    let restored = service
        .review_proposal(f.conversation, f.manuscript, proposal)
        .await
        .unwrap();
    assert_eq!(restored.id, proposal);
}

#[tokio::test]
async fn frozen_review_survives_copy_deletion_and_checks_scope_and_bytes() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let id = service
        .publish_workspace_changes(review.binding.clone(), review.changes.clone())
        .await
        .unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        request_id: review.binding.request_id,
    };
    let copy = service
        .restore_workspace(receipt, review.binding.context_snapshot_id)
        .await
        .unwrap();
    let identity = copy.lease.manifest().identity.clone();
    drop(copy);
    WorkspaceStore::open(
        &f.store.agent_workspace_directory().canonicalize().unwrap(),
        |_| Ok(()),
    )
    .unwrap()
    .discard(&identity)
    .unwrap();
    assert!(
        service
            .review_workspace(receipt, review.binding.context_snapshot_id)
            .await
            .is_err()
    );
    let frozen = service
        .review_proposal(f.conversation, f.manuscript, id)
        .await
        .unwrap();
    assert_eq!(frozen.id, id);
    assert_eq!(frozen.binding, review.binding);
    assert_eq!(frozen.changes, review.changes);
    assert_eq!(
        frozen.status,
        crate::agent::repository::change_sets::ChangeSetStatus::Pending
    );
    let file = service
        .review_proposal_file(f.conversation, f.manuscript, id, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(
        file.before,
        Some(ReviewContent::Text("Frozen manuscript".into()))
    );
    assert_eq!(
        file.after,
        Some(ReviewContent::Text("Revised manuscript".into()))
    );
    assert!(
        service
            .review_proposal(Uuid::new_v4(), f.manuscript, id)
            .await
            .is_err()
    );
    assert!(
        service
            .review_proposal(f.conversation, Uuid::new_v4(), id)
            .await
            .is_err()
    );
    assert!(
        service
            .review_proposal_file(f.conversation, f.manuscript, id, "../main.tex".into())
            .await
            .is_err()
    );
    let sha = &review.changes[0].after.as_ref().unwrap().sha256;
    let path: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select relative_path from content_objects where sha256=?1",
            [sha],
            |r| r.get(0),
        )
        .unwrap();
    std::fs::write(
        f.store.content_directory().join(path),
        b"different content!",
    )
    .unwrap();
    assert!(
        service
            .review_proposal_file(f.conversation, f.manuscript, id, "main.tex".into())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn missing_frozen_rows_and_removed_resource_bindings_are_rejected() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let id = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_change_sets set status='rejected' where id=?1",
            [id.to_string()],
        )
        .unwrap();
    assert_eq!(
        service
            .review_proposal(f.conversation, f.manuscript, id)
            .await
            .unwrap()
            .status,
        crate::agent::repository::change_sets::ChangeSetStatus::Rejected
    );
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(
        service
            .review_proposal(f.conversation, f.manuscript, id)
            .await
            .is_err()
    );
    f.store.connection().unwrap().execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![f.conversation.to_string(),f.manuscript.to_string()]).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_change_set_files where change_set_id=?1",
            [id.to_string()],
        )
        .unwrap();
    assert!(
        service
            .review_proposal(f.conversation, f.manuscript, id)
            .await
            .is_err()
    );
}
