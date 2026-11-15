use super::*;

#[path = "progress/rollback.rs"]
mod rollback;
#[path = "progress/startup.rs"]
mod startup;
use proposal_applications::{
    ApplicationPhase, ApplicationScope, ApplicationStatus, DispatchAdmission, FileReceipt,
    acknowledge_in, dispatch_in, finish_in, read_in, require_recovery_in,
};

async fn admitted() -> (
    crate::agent::writing_submission::tests::Fixture,
    ApplicationScope,
    FileReceipt,
) {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let receipt = FileReceipt {
        path: review.changes[0].path.clone(),
        after: review.changes[0].after.clone(),
        editor: None,
    };
    let proposal = service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
    let scope = ApplicationScope {
        conversation: f.conversation,
        manuscript: f.manuscript,
        proposal,
        operation: Uuid::new_v4(),
    };
    service
        .begin_proposal_application(
            f.conversation,
            proposal,
            scope.operation,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    (f, scope, receipt)
}

#[tokio::test]
async fn progress_requires_dispatch_and_exact_receipt_before_atomic_completion() {
    let (f, scope, mut receipt) = admitted().await;
    let mut db = f.store.connection().unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert!(acknowledge_in(&tx, scope, &receipt).is_err());
    assert!(finish_in(&mut tx, scope).is_err());
    assert!(dispatch_in(&tx, scope, "absent.tex").is_err());
    assert_eq!(
        dispatch_in(&tx, scope, &receipt.path).unwrap(),
        DispatchAdmission::NewIntent
    );
    assert_eq!(
        dispatch_in(&tx, scope, &receipt.path).unwrap(),
        DispatchAdmission::Existing(ApplicationPhase::Dispatched)
    );
    let valid = receipt.after.clone();
    receipt.after = None;
    assert!(acknowledge_in(&tx, scope, &receipt).is_err());
    receipt.after = valid;
    acknowledge_in(&tx, scope, &receipt).unwrap();
    acknowledge_in(&tx, scope, &receipt).unwrap();
    finish_in(&mut tx, scope).unwrap();
    finish_in(&mut tx, scope).unwrap();
    acknowledge_in(&tx, scope, &receipt).unwrap();
    assert!(require_recovery_in(&mut tx, scope).is_err());
    assert_eq!(
        dispatch_in(&tx, scope, &receipt.path).unwrap(),
        DispatchAdmission::Existing(ApplicationPhase::Acknowledged)
    );
    tx.commit().unwrap();
    let saved = read_in(
        &db,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .unwrap();
    assert_eq!(saved.status, ApplicationStatus::Applied);
    assert_eq!(
        db.query_row(
            "select status from agent_change_sets where id=?1",
            [scope.proposal.to_string()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "applied"
    );
    // Metadata fixtures never claim to have actually written the original.
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
async fn recovery_blocks_new_dispatch_and_keeps_unconfirmed_progress() {
    let (f, scope, receipt) = admitted().await;
    let mut db = f.store.connection().unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    require_recovery_in(&mut tx, scope).unwrap();
    require_recovery_in(&mut tx, scope).unwrap();
    assert!(dispatch_in(&tx, scope, &receipt.path).is_err());
    assert!(finish_in(&mut tx, scope).is_err());
    tx.commit().unwrap();
    let saved = read_in(
        &db,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .unwrap();
    assert_eq!(saved.status, ApplicationStatus::RecoveryRequired);
    assert_eq!(saved.files[0].phase, ApplicationPhase::Pending);
}

#[tokio::test]
async fn completion_failure_rolls_back_header_even_when_outer_transaction_commits() {
    let (f, scope, receipt) = admitted().await;
    let mut db = f.store.connection().unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    dispatch_in(&tx, scope, &receipt.path).unwrap();
    require_recovery_in(&mut tx, scope).unwrap();
    acknowledge_in(&tx, scope, &receipt).unwrap();
    tx.execute_batch("create trigger fail_finish before update of status on agent_change_sets when new.status='applied' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(finish_in(&mut tx, scope).is_err());
    tx.commit().unwrap();
    let saved = read_in(
        &db,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .unwrap();
    assert_eq!(saved.status, ApplicationStatus::RecoveryRequired);
    assert_eq!(saved.files[0].phase, ApplicationPhase::Acknowledged);
    assert_eq!(
        db.query_row(
            "select status from agent_change_sets where id=?1",
            [scope.proposal.to_string()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "recovery_required"
    );
}
