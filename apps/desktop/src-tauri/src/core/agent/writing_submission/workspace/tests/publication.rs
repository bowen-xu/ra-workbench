use super::*;
use crate::agent::workspace::ProposalObjects;
use research_agent_core::workspace::FileVersion;
use sha2::{Digest, Sha256};

#[path = "publication/apply_disk.rs"]
mod apply_disk;
#[path = "publication/apply_editor.rs"]
mod apply_editor;
#[path = "publication/current_editor_targets.rs"]
mod current_editor_targets;
#[path = "publication/deletion_managed.rs"]
mod deletion_managed;
#[cfg(target_os = "macos")]
#[path = "publication/delivery.rs"]
mod delivery;
#[path = "publication/editor_preflight.rs"]
mod editor_preflight;
#[path = "publication/restore_editor.rs"]
mod restore_editor;
#[path = "publication/rollback_plan.rs"]
mod rollback_plan;
#[path = "publication/saved_editor_source.rs"]
mod saved_editor_source;

fn fixture_with_objects() -> crate::agent::writing_submission::tests::Fixture {
    let f = fixture(true);
    // The shared fixture already runs the ordered migration chain. Replaying
    // individual SQL files would recreate tables and replace repaired guards.
    std::fs::create_dir_all(f.store.content_directory()).unwrap();
    f
}
async fn changed(f: &crate::agent::writing_submission::tests::Fixture) -> WorkspaceReview {
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
        copy.lease.working_directory().unwrap().join("main.tex"),
        b"Revised manuscript",
    )
    .unwrap();
    drop(copy);
    service
        .review_workspace(receipt, prepared.record.context_snapshot_id)
        .await
        .unwrap()
}

#[tokio::test]
async fn external_publication_stages_frozen_baseline_and_revision_bytes() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let after = review.changes[0].after.as_ref().unwrap();
    let payloads = vec![(after.sha256.clone(), b"Revised manuscript".to_vec())];
    let id = service
        .publish_external_changes(
            review.binding.clone(),
            review.changes.clone(),
            payloads.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .publish_external_changes(review.binding.clone(), review.changes.clone(), payloads)
            .await
            .unwrap(),
        id
    );
    let file = service
        .review_proposal_file(f.conversation, f.manuscript, id, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(
        file.before,
        Some(crate::agent::workspace::ReviewContent::Text(
            "Frozen manuscript".into()
        ))
    );
    assert_eq!(
        file.after,
        Some(crate::agent::workspace::ReviewContent::Text(
            "Revised manuscript".into()
        ))
    );
}

#[tokio::test]
async fn publication_commit_failure_fences_both_entry_points() {
    for external in [false, true] {
        let f = fixture_with_objects();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let review = changed(&f).await;
        f.store.connection().unwrap().execute_batch("pragma foreign_keys=on;
            create table test_publication_parent(id integer primary key);
            create table test_publication_child(parent_id integer references test_publication_parent(id) deferrable initially deferred);
            create trigger test_publication_commit after insert on agent_change_sets begin insert into test_publication_child(parent_id) values (1); end;").unwrap();
        let result = if external {
            let after = review.changes[0].after.as_ref().unwrap();
            service
                .publish_external_changes(
                    review.binding,
                    review.changes.clone(),
                    vec![(after.sha256.clone(), b"Revised manuscript".to_vec())],
                )
                .await
        } else {
            service
                .publish_workspace_changes(review.binding, review.changes)
                .await
        };
        assert!(matches!(result, Err(Error::RecoveryRequired)));
        assert!(
            f.store.connection().is_err(),
            "unconfirmed commit must fence the managed owner"
        );
    }
}

#[tokio::test]
async fn publication_rollback_failure_fences_managed_owner() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    {
        let db = f.store.connection().unwrap();
        db.execute_batch("create trigger test_publication_abort before insert on agent_change_sets begin select raise(abort,'synthetic failure'); end;").unwrap();
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    assert!(matches!(
        service
            .publish_workspace_changes(review.binding, review.changes)
            .await,
        Err(Error::RecoveryRequired)
    ));
    assert!(f.store.connection().is_err());
}

#[tokio::test]
async fn publication_stores_real_bytes_and_retains_sources_and_idempotency() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let id = service
        .publish_workspace_changes(review.binding.clone(), review.changes.clone())
        .await
        .unwrap();
    assert_eq!(
        service
            .publish_workspace_changes(review.binding.clone(), review.changes.clone())
            .await
            .unwrap(),
        id
    );
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_object_staging", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    for v in [&review.changes[0].before, &review.changes[0].after]
        .into_iter()
        .flatten()
    {
        let path: String = db
            .query_row(
                "select relative_path from content_objects where sha256=?1",
                [&v.sha256],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(f.store.content_directory().join(path)).unwrap())
            ),
            v.sha256
        );
        assert_eq!(
            db.query_row(
                "select count(*) from content_object_gc_candidates where content_sha256=?1",
                [&v.sha256],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
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
async fn final_publication_failure_keeps_verified_objects_collectable_and_retryable() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    f.store.connection().unwrap().execute_batch("create trigger fail_proposal before insert on agent_change_sets begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        service
            .publish_workspace_changes(review.binding.clone(), review.changes.clone())
            .await
            .is_err()
    );
    {
        let db = f.store.connection().unwrap();
        assert_eq!(
            db.query_row("select count(*) from agent_change_sets", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "select count(*) from content_object_gc_candidates",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        db.execute_batch("drop trigger fail_proposal").unwrap();
    }
    service
        .publish_workspace_changes(review.binding, review.changes)
        .await
        .unwrap();
}
#[tokio::test]
async fn journal_recovers_verified_bytes_but_keeps_missing_or_corrupt_writes_visible() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let objects =
        ProposalObjects::open(&f.store.content_directory().canonicalize().unwrap()).unwrap();
    for bytes in [
        b"complete".as_slice(),
        b"missing".as_slice(),
        b"corrupt".as_slice(),
    ] {
        let v = FileVersion {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            byte_size: bytes.len() as u64,
            executable: false,
        };
        f.store
            .connection()
            .unwrap()
            .execute(
                "insert into agent_object_staging(sha256,byte_size) values(?1,?2)",
                params![v.sha256, v.byte_size],
            )
            .unwrap();
        if bytes != b"missing" {
            let path = objects.stage(&v, bytes).unwrap();
            if bytes == b"corrupt" {
                std::fs::write(f.store.content_directory().join(path), b"partial").unwrap();
            }
        }
    }
    assert_eq!(service.recover_proposal_objects().await.unwrap(), (1, 2));
    assert_eq!(service.recover_proposal_objects().await.unwrap(), (0, 2));
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_object_staging", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "select count(*) from content_object_gc_candidates",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from agent_change_sets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn stale_inventory_or_scope_never_stages_objects() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let review = changed(&f).await;
    let mut wrong = review.binding.clone();
    wrong.context_snapshot_id = Uuid::new_v4();
    assert!(
        service
            .publish_workspace_changes(wrong, review.changes.clone())
            .await
            .is_err()
    );
    let mut stale = review.changes.clone();
    stale[0].after.as_mut().unwrap().sha256 = "a".repeat(64);
    assert!(
        service
            .publish_workspace_changes(review.binding, stale)
            .await
            .is_err()
    );
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_object_staging", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from content_objects", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[path = "publication/application.rs"]
mod application;
#[path = "publication/apply_added.rs"]
mod apply_added;
#[cfg(target_os = "macos")]
#[path = "publication/commands.rs"]
mod commands;
#[path = "publication/frozen.rs"]
mod frozen;
#[path = "publication/preflight.rs"]
mod preflight;
#[path = "publication/rejection.rs"]
mod rejection;
