use super::*;
use crate::agent::writing_submission::tests::{fixture, request};
#[cfg(target_os = "macos")]
#[path = "tests/commands.rs"]
mod commands;
#[cfg(target_os = "macos")]
#[path = "tests/consent.rs"]
mod consent;
#[cfg(target_os = "macos")]
#[path = "tests/owned_review.rs"]
mod owned_review;
#[cfg(target_os = "macos")]
#[path = "tests/queue_prepare_recovery.rs"]
mod queue_prepare_recovery;
#[path = "tests/review.rs"]
mod review;
#[cfg(target_os = "macos")]
#[path = "tests/review_commands.rs"]
mod review_commands;

#[tokio::test]
async fn prepare_restore_preserves_frozen_copy_and_never_implies_consent() {
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
    let copy = service
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
    assert_eq!(
        std::fs::read(copy.lease.working_directory().unwrap().join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    let binding = copy.binding.clone();
    assert!(
        execution_workspaces::authorize(
            &f.store.connection().unwrap(),
            &binding,
            copy.lease.manifest()
        )
        .is_err()
    );
    drop(copy);
    std::fs::write(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex"),
        b"later edits",
    )
    .unwrap();
    let restored = service.restore_workspace(receipt, context).await.unwrap();
    assert_eq!(restored.binding, binding);
    assert_eq!(
        std::fs::read(restored.lease.working_directory().unwrap().join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    drop(restored);
    assert!(
        service
            .prepare_workspace(
                receipt,
                context,
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![],
                }
            )
            .await
            .is_err()
    );
    assert!(
        service
            .restore_workspace(receipt, Uuid::new_v4())
            .await
            .is_err()
    );
    {
        let mut db = f.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        execution_workspaces::revoke_in(&tx, receipt.conversation_id, receipt.request_id).unwrap();
        tx.commit().unwrap();
    }
    assert!(service.restore_workspace(receipt, context).await.is_err());
}

#[tokio::test]
async fn stale_context_rejects_before_creating_storage_and_bad_snapshot_cleans_copy() {
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
    assert!(
        service
            .prepare_workspace(
                receipt,
                Uuid::new_v4(),
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![],
                }
            )
            .await
            .is_err()
    );
    assert!(!f.store.agent_workspace_directory().exists());
    // Unconfirmed editor text conflicts with the immutable disclosed input.
    assert!(
        service
            .prepare_workspace(
                receipt,
                prepared.record.context_snapshot_id,
                WritingWorkspaceRequest {
                    manuscript_id: f.manuscript,
                    buffers: vec![crate::agent::writing_workspace::EditorBuffer {
                        path: "main.tex".into(),
                        expected_disk_sha256: f.hash.clone(),
                        editor: research_agent_core::workspace::EditorVersion {
                            session_id: Uuid::new_v4(),
                            revision: Uuid::new_v4()
                        },
                        text: "different buffer".into(),
                    }],
                }
            )
            .await
            .is_err()
    );
    let remaining = std::fs::read_dir(f.store.agent_workspace_directory())
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(remaining.len(), 1);
    let receipt_file = &remaining[0];
    assert!(receipt_file.file_type().unwrap().is_file());
    let completion: serde_json::Value =
        serde_json::from_slice(&std::fs::read(receipt_file.path()).unwrap()).unwrap();
    let identity: research_agent_core::workspace::WorkspaceIdentity =
        serde_json::from_value(completion["identity"].clone()).unwrap();
    assert_eq!(identity.conversation_id, receipt.conversation_id);
    assert_eq!(
        receipt_file.file_name().to_str().unwrap(),
        format!("discarded-{}.json", identity.lease_id)
    );
    assert!(
        execution_workspaces::read(
            &f.store.connection().unwrap(),
            receipt.conversation_id,
            receipt.request_id
        )
        .unwrap()
        .is_none()
    );
}

#[path = "tests/publication.rs"]
mod publication;

#[cfg(target_os = "macos")]
#[path = "tests/successor_source.rs"]
mod successor_source;
