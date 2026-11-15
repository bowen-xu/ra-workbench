use super::*;
use crate::agent::repository::{change_sets, execution_workspaces::discard};

#[tokio::test]
async fn undo_and_frozen_content_survive_execution_files_and_binding_removal() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let original = existing(&f, false).await;
    let applied = service
        .apply_disk_proposal_file(original, "main.tex".into())
        .await
        .unwrap();
    let frozen = service
        .review_proposal(f.conversation, f.manuscript, original.proposal)
        .await
        .unwrap();
    let content = service
        .review_proposal_file(
            f.conversation,
            f.manuscript,
            original.proposal,
            "main.tex".into(),
        )
        .await
        .unwrap();
    let binding = frozen.binding.clone();
    let identity = {
        let db = f.store.connection().unwrap();
        execution_workspaces::read_for_review(&db, binding.conversation_id, binding.request_id)
            .unwrap()
            .unwrap()
            .1
            .identity
    };
    let operation = discard::Request {
        operation_id: Uuid::new_v4(),
        binding,
        environment_id: identity.environment_id.clone(),
        inventory_hash: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&frozen.changes).unwrap())
        ),
    };
    let copies = open(&f.store).unwrap();
    let held = copies.restore(&identity).unwrap();
    assert_eq!(held.changes().unwrap(), frozen.changes);
    {
        let mut db = f.store.connection().unwrap();
        let mut tx = db.transaction().unwrap();
        discard::begin_in(&mut tx, &operation).unwrap();
        tx.commit().unwrap();
    }
    drop(held);
    copies.discard(&identity).unwrap();
    // Simulate an acknowledgement gap: the filesystem receipt can be read again
    // before the database completion transaction is retried.
    copies.discard(&identity).unwrap();
    {
        let mut db = f.store.connection().unwrap();
        let mut tx = db.transaction().unwrap();
        discard::complete_in(&mut tx, &operation).unwrap();
        tx.commit().unwrap();
        assert_eq!(
            change_sets::read_in(&db, f.conversation, f.manuscript, original.proposal)
                .unwrap()
                .changes,
            frozen.changes
        );
    }
    let after = service
        .review_proposal_file(
            f.conversation,
            f.manuscript,
            original.proposal,
            "main.tex".into(),
        )
        .await
        .unwrap();
    assert_eq!(after.before, content.before);
    assert_eq!(after.after, content.after);
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
}
