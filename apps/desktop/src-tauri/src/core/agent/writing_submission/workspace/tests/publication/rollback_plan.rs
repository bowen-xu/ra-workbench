use super::*;
use crate::agent::repository::proposal_applications::{
    ApplicationPhase, ApplicationScope, ApplicationStatus,
};
use crate::agent::writing_application_commands as boundary;

fn rollback_request(scope: ApplicationScope) -> boundary::Request {
    serde_json::from_value(serde_json::json!({"action":"rollbackDisk","scope":{
        "conversationId":scope.conversation,"manuscriptId":scope.manuscript,
        "proposalId":scope.proposal,"operationId":scope.operation
    }}))
    .unwrap()
}

#[tokio::test]
async fn mixed_disk_plan_resumes_partial_rollback_and_restores_original_inventory() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source");
    let original = std::fs::read(source.join("main.tex")).unwrap();
    std::fs::write(source.join("old.bin"), b"old bytes").unwrap();
    {
        let mut db = f.store.connection().unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        f.store
            .register_created_writing_file_in(
                &tx,
                f.manuscript,
                "old.bin",
                &FileVersion {
                    sha256: format!("{:x}", Sha256::digest(b"old bytes")),
                    byte_size: 9,
                    executable: false,
                },
            )
            .unwrap();
        tx.commit().unwrap();
    }
    let prepared = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
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
    let work = copy.lease.working_directory().unwrap();
    std::fs::write(work.join("new.bin"), b"new bytes").unwrap();
    std::fs::write(work.join("main.tex"), b"replacement").unwrap();
    std::fs::remove_file(work.join("old.bin")).unwrap();
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
    for path in ["new.bin", "main.tex"] {
        assert_eq!(
            service
                .apply_disk_proposal_file(scope, path.into())
                .await
                .unwrap()
                .status,
            ApplicationStatus::Applying
        );
    }
    f.store.connection().unwrap().execute_batch("create trigger fail_delete before delete on manuscript_files begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .apply_disk_proposal_file(scope, "old.bin".into())
            .await
            .is_err()
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_delete;")
        .unwrap();
    // Reverse order restores old.bin then new.bin; main.tex conflicts last.
    std::fs::write(source.join("main.tex"), b"user edit").unwrap();
    assert!(
        boundary::execute(&f.store, rollback_request(scope))
            .await
            .is_err()
    );
    let partial = service
        .read_proposal_application(
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )
        .await
        .unwrap();
    assert_eq!(partial.status, ApplicationStatus::RecoveryRequired);
    assert_eq!(
        partial
            .files
            .iter()
            .filter(|file| file.phase == ApplicationPhase::Restored)
            .count(),
        2
    );
    assert_eq!(std::fs::read(source.join("old.bin")).unwrap(), b"old bytes");
    assert!(!source.join("new.bin").exists());
    assert_eq!(
        std::fs::read(source.join("main.tex")).unwrap(),
        b"user edit"
    );
    std::fs::write(source.join("main.tex"), b"replacement").unwrap();
    let response = serde_json::to_value(
        boundary::execute(&f.store, rollback_request(scope))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response["status"], "rolledBack");
    let complete = service
        .read_proposal_application(
            scope.conversation,
            scope.manuscript,
            scope.proposal,
            scope.operation,
        )
        .await
        .unwrap();
    assert_eq!(complete.status, ApplicationStatus::RolledBack);
    assert!(
        complete
            .files
            .iter()
            .all(|file| file.phase == ApplicationPhase::Restored)
    );
    assert_eq!(std::fs::read(source.join("main.tex")).unwrap(), original);
    let count:i64 = f.store.connection().unwrap().query_row("select count(*) from manuscript_files where relative_path in ('main.tex','old.bin','new.bin')",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        service.rollback_disk_proposal(scope).await.unwrap(),
        complete
    );
    std::fs::write(source.join("main.tex"), b"later edit").unwrap();
    assert_eq!(
        serde_json::to_value(
            boundary::execute(&f.store, rollback_request(scope))
                .await
                .unwrap()
        )
        .unwrap(),
        response
    );
    assert_eq!(
        std::fs::read(source.join("main.tex")).unwrap(),
        b"later edit"
    );
}
