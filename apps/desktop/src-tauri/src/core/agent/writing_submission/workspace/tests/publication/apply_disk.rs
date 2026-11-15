use super::*;
#[path = "apply_disk/application_discovery.rs"]
mod application_discovery;
#[path = "apply_disk/commands.rs"]
mod commands;
#[path = "apply_disk/deleted_chat.rs"]
pub(super) mod deleted_chat;
#[path = "apply_disk/discarded_workspace.rs"]
mod discarded_workspace;
#[path = "apply_disk/rollback_removed.rs"]
mod rollback_removed;
#[path = "apply_disk/rollback_replaced.rs"]
mod rollback_replaced;
#[path = "apply_disk/undo_cancel.rs"]
mod undo_cancel;
#[path = "apply_disk/undo_commands.rs"]
mod undo_commands;
#[path = "apply_disk/undo_discovery.rs"]
mod undo_discovery;
#[path = "apply_disk/undo_execution.rs"]
mod undo_execution;
#[path = "apply_disk/undo_file_ownership.rs"]
mod undo_file_ownership;
#[path = "apply_disk/undo_preparation.rs"]
mod undo_preparation;
#[path = "apply_disk/undo_preview.rs"]
mod undo_preview;
#[path = "apply_disk/undo_progress.rs"]
mod undo_progress;
#[path = "apply_disk/undo_resume.rs"]
mod undo_resume;
#[path = "apply_disk/undo_rollback.rs"]
mod undo_rollback;
#[path = "apply_disk/undo_rollback_plan.rs"]
mod undo_rollback_plan;
use crate::agent::repository::proposal_applications::{
    ApplicationPhase, ApplicationScope, ApplicationStatus,
};

pub(super) async fn existing(
    f: &crate::agent::writing_submission::tests::Fixture,
    remove: bool,
) -> ApplicationScope {
    existing_with_extra(f, remove, false).await
}

async fn existing_with_extra(
    f: &crate::agent::writing_submission::tests::Fixture,
    remove: bool,
    extra: bool,
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
    let path = copy.lease.working_directory().unwrap().join("main.tex");
    if remove {
        std::fs::remove_file(path).unwrap();
    } else {
        std::fs::write(path, b"replacement").unwrap();
    }
    if extra {
        std::fs::write(
            copy.lease.working_directory().unwrap().join("other.tex"),
            b"extra",
        )
        .unwrap();
    }
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

fn source(f: &crate::agent::writing_submission::tests::Fixture) -> std::path::PathBuf {
    f.store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source")
}

#[tokio::test]
async fn replaces_or_removes_and_confirms_index_with_original_retained() {
    for remove in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let original = std::fs::read(source(&f).join("main.tex")).unwrap();
        let scope = existing(&f, remove).await;
        let saved = service
            .apply_disk_proposal_file(scope, "main.tex".into())
            .await
            .unwrap();
        assert_eq!(saved.status, ApplicationStatus::Applied);
        assert_eq!(saved.files[0].phase, ApplicationPhase::Acknowledged);
        let count: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_files where relative_path='main.tex'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, if remove { 0 } else { 1 });
        if remove {
            assert!(!source(&f).join("main.tex").exists());
        } else {
            assert_eq!(
                std::fs::read(source(&f).join("main.tex")).unwrap(),
                b"replacement"
            );
            let hash: String = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "select sha256 from manuscript_files where relative_path='main.tex'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(hash, format!("{:x}", Sha256::digest(b"replacement")));
        }
        let backup = std::fs::read_dir(source(&f))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|ext| ext == "bak"))
            .unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), original);
        std::fs::write(source(&f).join("main.tex"), b"later user file").unwrap();
        assert_eq!(
            service
                .apply_disk_proposal_file(scope, "main.tex".into())
                .await
                .unwrap(),
            saved
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"later user file"
        );
    }
}

#[tokio::test]
async fn index_failure_keeps_intent_and_original_without_replaying_mutation() {
    for remove in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let scope = existing(&f, remove).await;
        let event = if remove { "delete" } else { "update" };
        f.store.connection().unwrap().execute_batch(&format!("create trigger fail_index before {event} on manuscript_files begin select raise(abort,'fixture'); end;")).unwrap();
        assert!(matches!(
            service
                .apply_disk_proposal_file(scope, "main.tex".into())
                .await,
            Err(Error::RecoveryRequired)
        ));
        let saved = service
            .read_proposal_application(
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )
            .await
            .unwrap();
        assert_eq!(saved.status, ApplicationStatus::RecoveryRequired);
        assert_eq!(saved.files[0].phase, ApplicationPhase::Dispatched);
        let count: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_files where relative_path='main.tex'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        std::fs::write(source(&f).join("main.tex"), b"user correction").unwrap();
        assert_eq!(
            service
                .apply_disk_proposal_file(scope, "main.tex".into())
                .await
                .unwrap(),
            saved
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"user correction"
        );
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_index;")
            .unwrap();
        assert!(
            service
                .reconcile_disk_proposal_file(scope, "main.tex".into())
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"user correction"
        );
        // Explicitly restore the expected result, then reconcile without replay.
        if remove {
            std::fs::remove_file(source(&f).join("main.tex")).unwrap();
        } else {
            std::fs::write(source(&f).join("main.tex"), b"replacement").unwrap();
        }
        let recovered = service
            .reconcile_disk_proposal_file(scope, "main.tex".into())
            .await
            .unwrap();
        assert_eq!(recovered.status, ApplicationStatus::Applied);
        assert_eq!(recovered.files[0].phase, ApplicationPhase::Acknowledged);
        let count: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_files where relative_path='main.tex'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, if remove { 0 } else { 1 });
        std::fs::write(source(&f).join("main.tex"), b"later edit").unwrap();
        assert_eq!(
            service
                .reconcile_disk_proposal_file(scope, "main.tex".into())
                .await
                .unwrap(),
            recovered
        );
        assert_eq!(
            std::fs::read(source(&f).join("main.tex")).unwrap(),
            b"later edit"
        );
    }
}

#[tokio::test]
async fn recovery_rejects_missing_or_changed_backup_even_when_target_matches() {
    use crate::agent::repository::proposal_applications::dispatch_in;
    for remove in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let scope = existing(&f, remove).await;
        assert!(
            service
                .reconcile_disk_proposal_file(scope, "main.tex".into())
                .await
                .is_err()
        );
        {
            let mut db = f.store.connection().unwrap();
            let tx = db
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            dispatch_in(&tx, scope, "main.tex").unwrap();
            tx.commit().unwrap();
        }
        // Construct an incomplete dispatched operation: target result exists,
        // but the original was not durably retained by this operation.
        if remove {
            std::fs::remove_file(source(&f).join("main.tex")).unwrap();
        } else {
            std::fs::write(source(&f).join("main.tex"), b"replacement").unwrap();
        }
        assert!(
            service
                .reconcile_disk_proposal_file(scope, "main.tex".into())
                .await
                .is_err()
        );
        let backup = source(&f).join(format!(
            ".agent-before-{}-{:x}.bak",
            scope.operation,
            Sha256::digest(b"main.tex")
        ));
        std::fs::write(&backup, b"unrelated bytes").unwrap();
        assert!(
            service
                .reconcile_disk_proposal_file(scope, "main.tex".into())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(backup).unwrap(), b"unrelated bytes");
        let saved = service
            .read_proposal_application(
                scope.conversation,
                scope.manuscript,
                scope.proposal,
                scope.operation,
            )
            .await
            .unwrap();
        assert_eq!(saved.status, ApplicationStatus::RecoveryRequired);
        assert_eq!(saved.files[0].phase, ApplicationPhase::Dispatched);
    }
}

#[tokio::test]
async fn rejects_changed_disk_or_index_before_dispatch() {
    for change_disk in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let scope = existing(&f, false).await;
        if change_disk {
            std::fs::write(source(&f).join("main.tex"), b"external edit").unwrap();
        } else {
            f.store.connection().unwrap().execute("update manuscript_files set byte_size=byte_size+1 where relative_path='main.tex'", []).unwrap();
        }
        assert!(
            service
                .apply_disk_proposal_file(scope, "main.tex".into())
                .await
                .is_err()
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
        assert_eq!(saved.files[0].phase, ApplicationPhase::Pending);
        assert!(std::fs::read_dir(source(&f)).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".agent-before-")
        }));
    }
}
