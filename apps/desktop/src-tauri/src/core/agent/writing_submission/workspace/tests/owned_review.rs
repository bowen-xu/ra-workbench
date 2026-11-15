//! Sharing the current owner must not weaken review authority or byte checks.
use super::*;
use crate::agent::workspace::ReviewContent;

#[tokio::test]
async fn owned_review_checks_binding_versions_and_uncertain_writer_intent() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let prepared = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let context = prepared.record.context_snapshot_id;
    let mut copy = service
        .prepare_workspace(
            receipt,
            context,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    std::fs::write(
        copy.lease.working_directory().unwrap().join("main.tex"),
        b"Owned revised manuscript",
    )
    .unwrap();
    assert!(service.review_workspace(receipt, context).await.is_err());

    let mut connection = f.store.connection().unwrap();
    let db = connection.transaction().unwrap();
    let review = service
        .review_owned_workspace_binding(&db, &copy.binding, &copy.lease)
        .unwrap();
    assert_eq!(review.binding, copy.binding);
    assert_eq!(review.changes.len(), 1);
    let expected = review.changes[0].clone();
    let content = service
        .read_owned_workspace_change_binding(&db, &copy.binding, &copy.lease, expected.clone())
        .unwrap();
    assert_eq!(
        content.before,
        Some(ReviewContent::Text("Frozen manuscript".into()))
    );
    assert_eq!(
        content.after,
        Some(ReviewContent::Text("Owned revised manuscript".into()))
    );
    let mut wrong_context = copy.binding.clone();
    wrong_context.context_snapshot_id = Uuid::new_v4();
    assert!(
        service
            .review_owned_workspace_binding(&db, &wrong_context, &copy.lease)
            .is_err()
    );
    let mut wrong_receipt = copy.binding.clone();
    wrong_receipt.request_id = Uuid::new_v4();
    assert!(
        service
            .review_owned_workspace_binding(&db, &wrong_receipt, &copy.lease)
            .is_err()
    );
    let mut stale = expected;
    stale.after.as_mut().unwrap().sha256 = "0".repeat(64);
    assert!(
        service
            .read_owned_workspace_change_binding(&db, &copy.binding, &copy.lease, stale)
            .is_err()
    );

    // Revocation removes execution authority, while the original read authority
    // remains. Archiving removes the live conversation binding used by tools.
    execution_workspaces::revoke_in(&db, receipt.conversation_id, receipt.request_id).unwrap();
    assert!(execution_workspaces::authorize(&db, &copy.binding, copy.lease.manifest()).is_err());
    assert!(
        service
            .review_owned_workspace_binding(&db, &copy.binding, &copy.lease)
            .is_ok()
    );
    db.execute(
        "update agent_conversations set archived=1 where id=?1",
        [f.conversation.to_string()],
    )
    .unwrap();
    assert!(
        service
            .review_owned_workspace_binding(&db, &copy.binding, &copy.lease)
            .is_err()
    );
    db.execute(
        "update agent_conversations set archived=0 where id=?1",
        [f.conversation.to_string()],
    )
    .unwrap();

    let intent = copy.lease.begin_execution().unwrap();
    assert!(matches!(
        service.review_owned_workspace_binding(&db, &copy.binding, &copy.lease),
        Err(Error::Lease(LeaseError::Busy))
    ));
    drop(intent); // An uncertain intent intentionally remains; no process was launched.
    assert!(matches!(
        service.review_owned_workspace_binding(&db, &copy.binding, &copy.lease),
        Err(Error::Lease(LeaseError::Busy))
    ));
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
