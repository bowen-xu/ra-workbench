use super::*;
use proposal_applications::{acknowledge_disk_restored_in, finish_rollback_in};

#[tokio::test]
async fn completed_application_cannot_be_reopened_as_interrupted_rollback() {
    let (f, scope, receipt) = admitted().await;
    let mut db = f.store.connection().unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let proposal = crate::agent::repository::change_sets::read_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )
    .unwrap();
    dispatch_in(&tx, scope, &receipt.path).unwrap();
    acknowledge_in(&tx, scope, &receipt).unwrap();
    finish_in(&mut tx, scope).unwrap();
    assert!(require_recovery_in(&mut tx, scope).is_err());
    assert!(
        acknowledge_disk_restored_in(
            &tx,
            scope,
            &receipt.path,
            proposal.changes[0].before.as_ref()
        )
        .is_err()
    );
    assert!(finish_rollback_in(&mut tx, scope).is_err());
    tx.commit().unwrap();
    assert_eq!(
        read_in(
            &db,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation
        )
        .unwrap()
        .status,
        ApplicationStatus::Applied
    );
}

#[tokio::test]
async fn rollback_requires_recovery_and_exact_original_receipt() {
    let (f, scope, after) = admitted().await;
    let mut db = f.store.connection().unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let proposal = crate::agent::repository::change_sets::read_in(
        &tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )
    .unwrap();
    let before = proposal.changes[0].before.as_ref();
    assert!(finish_rollback_in(&mut tx, scope).is_err());
    dispatch_in(&tx, scope, &after.path).unwrap();
    assert!(acknowledge_disk_restored_in(&tx, scope, &after.path, before).is_err());
    require_recovery_in(&mut tx, scope).unwrap();
    assert!(finish_rollback_in(&mut tx, scope).is_err());
    assert!(acknowledge_disk_restored_in(&tx, scope, &after.path, after.after.as_ref()).is_err());
    assert!(
        acknowledge_disk_restored_in(
            &tx,
            ApplicationScope {
                conversation: Uuid::new_v4(),
                ..scope
            },
            &after.path,
            before
        )
        .is_err()
    );
    acknowledge_disk_restored_in(&tx, scope, &after.path, before).unwrap();
    acknowledge_disk_restored_in(&tx, scope, &after.path, before).unwrap();
    assert!(dispatch_in(&tx, scope, &after.path).is_err());
    finish_rollback_in(&mut tx, scope).unwrap();
    finish_rollback_in(&mut tx, scope).unwrap();
    tx.commit().unwrap();
    let saved = read_in(
        &db,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
    .unwrap();
    assert_eq!(saved.status, ApplicationStatus::RolledBack);
    assert_eq!(saved.files[0].phase, ApplicationPhase::Restored);
}

#[tokio::test]
async fn rollback_terminal_transition_is_atomic_even_if_outer_transaction_commits() {
    let (f, scope, _) = admitted().await;
    let mut db = f.store.connection().unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    require_recovery_in(&mut tx, scope).unwrap();
    tx.execute_batch("create trigger fail_rollback before update of status on agent_change_sets when new.status='undone' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(finish_rollback_in(&mut tx, scope).is_err());
    tx.commit().unwrap();
    assert_eq!(
        read_in(
            &db,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation
        )
        .unwrap()
        .status,
        ApplicationStatus::RecoveryRequired
    );
    db.execute_batch("drop trigger fail_rollback;").unwrap();
    let mut tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    finish_rollback_in(&mut tx, scope).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        read_in(
            &db,
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation
        )
        .unwrap()
        .status,
        ApplicationStatus::RolledBack
    );
}
