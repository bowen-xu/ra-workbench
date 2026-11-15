use super::*;
#[path = "apply_added/rollback.rs"]
mod rollback;
#[path = "apply_added/undo_preview.rs"]
mod undo_preview;
use crate::agent::repository::proposal_applications::{
    ApplicationPhase, ApplicationScope, ApplicationStatus,
};

pub(super) async fn added(
    f: &crate::agent::writing_submission::tests::Fixture,
    bytes: &[u8],
) -> ApplicationScope {
    let service = WritingSubmissionService::from_app_store(&f.store);
    let prepared = service
        .prepare(&f.models, request(f, Uuid::new_v4()))
        .await
        .unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: prepared.record.request_id,
        manuscript_id: f.manuscript,
    };
    let copy = service
        .prepare_workspace(
            receipt,
            prepared.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    std::fs::write(
        copy.lease.working_directory().unwrap().join("new.bin"),
        bytes,
    )
    .unwrap();
    drop(copy);
    let review = service
        .review_workspace(receipt, prepared.record.context_snapshot_id)
        .await
        .unwrap();
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
    scope
}

#[tokio::test]
async fn adds_full_binary_bytes_and_atomically_indexes_acknowledges_and_finishes() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let bytes = vec![0xff; 1024 * 1024 + 1];
    let scope = added(&f, &bytes).await;
    let result = service
        .apply_added_proposal_file(scope, "new.bin".into())
        .await
        .unwrap();
    assert_eq!(result.status, ApplicationStatus::Applied);
    assert_eq!(result.files[0].phase, ApplicationPhase::Acknowledged);
    let destination = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/new.bin");
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    let (size, hash):(u64,String) = f.store.connection().unwrap().query_row("select byte_size,sha256 from manuscript_files where manuscript_id=?1 and relative_path='new.bin'", [f.manuscript.to_string()], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(size, bytes.len() as u64);
    assert_eq!(hash, format!("{:x}", Sha256::digest(&bytes)));
    // A retry returns the historical result, never overwrites later user work.
    std::fs::write(&destination, b"later user work").unwrap();
    assert_eq!(
        service
            .apply_added_proposal_file(scope, "new.bin".into())
            .await
            .unwrap(),
        result
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"later user work");
}

#[tokio::test]
async fn index_failure_retains_created_bytes_and_dispatched_recovery_record() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let scope = added(&f, b"new bytes").await;
    f.store.connection().unwrap().execute_batch("create trigger fail_index before insert on manuscript_files when new.relative_path='new.bin' begin select raise(abort,'fixture'); end;").unwrap();
    assert!(matches!(
        service
            .apply_added_proposal_file(scope, "new.bin".into())
            .await,
        Err(Error::RecoveryRequired)
    ));
    let destination = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/new.bin");
    assert_eq!(std::fs::read(&destination).unwrap(), b"new bytes");
    let result = service
        .read_proposal_application(
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )
        .await
        .unwrap();
    assert_eq!(result.status, ApplicationStatus::RecoveryRequired);
    assert_eq!(result.files[0].phase, ApplicationPhase::Dispatched);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_files where relative_path='new.bin'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    std::fs::write(&destination, b"user correction").unwrap();
    assert_eq!(
        service
            .apply_added_proposal_file(scope, "new.bin".into())
            .await
            .unwrap(),
        result
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"user correction");
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_index;")
        .unwrap();
    assert!(
        service
            .reconcile_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"user correction");
    // Simulate the user restoring the intended content before explicit recovery.
    std::fs::write(&destination, b"new bytes").unwrap();
    // Simulate a crash after hard-link publication but before staging cleanup.
    let stage = destination.parent().unwrap().join(format!(
        ".agent-create-{}-{:x}.tmp",
        scope.operation,
        Sha256::digest(b"new.bin")
    ));
    std::fs::hard_link(&destination, &stage).unwrap();
    let recovered = service
        .reconcile_added_proposal_file(scope, "new.bin".into())
        .await
        .unwrap();
    assert_eq!(recovered.status, ApplicationStatus::Applied);
    assert!(!stage.exists());
    assert_eq!(recovered.files[0].phase, ApplicationPhase::Acknowledged);
    let (size, hash):(u64,String) = f.store.connection().unwrap().query_row("select byte_size,sha256 from manuscript_files where manuscript_id=?1 and relative_path='new.bin'", [f.manuscript.to_string()], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(size, 9);
    assert_eq!(hash, format!("{:x}", Sha256::digest(b"new bytes")));
    std::fs::write(&destination, b"later edit").unwrap();
    assert_eq!(
        service
            .reconcile_added_proposal_file(scope, "new.bin".into())
            .await
            .unwrap(),
        recovered
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"later edit");
}

#[tokio::test]
async fn reconciliation_requires_dispatch_and_never_recreates_a_missing_file() {
    use crate::agent::repository::proposal_applications::dispatch_in;
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let scope = added(&f, b"new bytes").await;
    assert!(
        service
            .reconcile_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    {
        let mut db = f.store.connection().unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        dispatch_in(&tx, scope, "new.bin").unwrap();
        tx.commit().unwrap();
    }
    assert!(
        service
            .reconcile_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    let destination = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/new.bin");
    assert!(!destination.exists());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_files where relative_path='new.bin'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let saved = service
        .read_proposal_application(
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )
        .await
        .unwrap();
    assert_eq!(saved.files[0].phase, ApplicationPhase::Dispatched);
    assert_eq!(saved.status, ApplicationStatus::RecoveryRequired);
    assert!(
        service
            .reconcile_added_proposal_file(
                ApplicationScope {
                    conversation: Uuid::new_v4(),
                    ..scope
                },
                "new.bin".into()
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rejects_wrong_scope_or_occupied_destination_before_dispatch() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let scope = added(&f, b"new bytes").await;
    assert!(
        service
            .apply_added_proposal_file(
                ApplicationScope {
                    conversation: Uuid::new_v4(),
                    ..scope
                },
                "new.bin".into()
            )
            .await
            .is_err()
    );
    let destination = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/new.bin");
    assert!(!destination.exists());
    std::fs::write(&destination, b"existing user file").unwrap();
    assert!(
        service
            .apply_added_proposal_file(scope, "new.bin".into())
            .await
            .is_err()
    );
    let result = service
        .read_proposal_application(
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )
        .await
        .unwrap();
    assert_eq!(result.files[0].phase, ApplicationPhase::Pending);
    assert_eq!(std::fs::read(destination).unwrap(), b"existing user file");
}
