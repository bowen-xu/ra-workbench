use super::*;
use crate::agent::{
    service::AgentSessionService,
    writing_submission::tests::{Fixture, fixture, request},
};
use std::sync::Arc;
#[path = "tests/commands.rs"]
mod commands;

async fn prepared() -> (Fixture, journal::Request) {
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
        b"changed",
    )
    .unwrap();
    let changes = copy.lease.changes().unwrap();
    let operation = journal::Request {
        operation_id: Uuid::new_v4(),
        binding: copy.binding.clone(),
        environment_id: copy.lease.manifest().identity.environment_id.clone(),
        inventory_hash: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&changes).unwrap())
        ),
    };
    drop(copy);
    (f, operation)
}
async fn discard(f: &Fixture, operation: journal::Request) -> Result<(), Error> {
    Arc::new(AgentSessionService::from_app_store(&f.store).unwrap())
        .discard_workspace(
            WritingSubmissionService::from_app_store(&f.store),
            operation,
        )
        .await
}
fn state(f: &Fixture, operation: &journal::Request) -> Option<journal::State> {
    journal::read(&f.store.connection().unwrap(), operation).unwrap()
}
fn begin(f: &Fixture, operation: &journal::Request) {
    let mut db = f.store.connection().unwrap();
    let mut tx = db.transaction().unwrap();
    journal::begin_in(&mut tx, operation).unwrap();
    tx.commit().unwrap();
}

#[tokio::test]
async fn coordinated_discard_preserves_source_and_fences_repeat_preparation() {
    let (f, operation) = prepared().await;
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    let original = std::fs::read(&source).unwrap();
    // Private execution ownership remains valid after UI archival/unbinding.
    f.store
        .connection()
        .unwrap()
        .execute_batch(
            "update agent_conversations set archived=1; delete from agent_conversation_bindings;",
        )
        .unwrap();
    discard(&f, operation.clone()).await.unwrap();
    assert_eq!(state(&f, &operation), Some(journal::State::Complete));
    assert!(
        !f.store
            .agent_workspace_directory()
            .join(operation.binding.lease_id.to_string())
            .exists()
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    discard(&f, operation.clone()).await.unwrap();
    // Complete replay is still valid after the owning conversation is deleted.
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversations", [])
        .unwrap();
    discard(&f, operation.clone()).await.unwrap();
    assert!(
        journal::ensure_preparation_allowed(
            &f.store.connection().unwrap(),
            operation.binding.conversation_id,
            operation.binding.request_id
        )
        .is_err()
    );
}

#[tokio::test]
async fn changed_inventory_and_held_lease_do_not_start_cleanup() {
    let (f, operation) = prepared().await;
    let copies = open(&f.store).unwrap();
    let held = copies.restore(&operation.identity()).unwrap();
    assert!(discard(&f, operation.clone()).await.is_err());
    assert_eq!(state(&f, &operation), None);
    std::fs::write(held.working_directory().unwrap().join("main.tex"), b"later").unwrap();
    drop(held);
    assert!(matches!(
        discard(&f, operation.clone()).await,
        Err(Error::Lease(LeaseError::Conflict))
    ));
    assert_eq!(state(&f, &operation), None);
    assert_eq!(copies.discard_state(&operation.identity()).unwrap(), None);
    let held = copies.restore(&operation.identity()).unwrap();
    assert_eq!(
        std::fs::read(held.working_directory().unwrap().join("main.tex")).unwrap(),
        b"later"
    );
}

#[tokio::test]
async fn failed_database_completion_retries_using_filesystem_receipt() {
    let (f, operation) = prepared().await;
    f.store.connection().unwrap().execute_batch("create trigger fail_release before delete on agent_execution_workspaces begin select raise(abort,'fixture'); end;").unwrap();
    assert!(discard(&f, operation.clone()).await.is_err());
    assert_eq!(state(&f, &operation), Some(journal::State::Pending));
    assert_eq!(
        open(&f.store)
            .unwrap()
            .discard_state(&operation.identity())
            .unwrap(),
        Some(DiscardState::Complete)
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_release;")
        .unwrap();
    discard(&f, operation.clone()).await.unwrap();
    assert_eq!(state(&f, &operation), Some(journal::State::Complete));
}

#[tokio::test]
async fn intent_without_filesystem_journal_rechecks_review_before_deleting() {
    let (f, operation) = prepared().await;
    begin(&f, &operation);
    let copies = open(&f.store).unwrap();
    let held = copies.restore(&operation.identity()).unwrap();
    std::fs::write(held.working_directory().unwrap().join("main.tex"), b"later").unwrap();
    drop(held);
    assert!(matches!(
        discard(&f, operation.clone()).await,
        Err(Error::Lease(LeaseError::Conflict))
    ));
    assert_eq!(copies.discard_state(&operation.identity()).unwrap(), None);
    let held = copies.restore(&operation.identity()).unwrap();
    std::fs::write(
        held.working_directory().unwrap().join("main.tex"),
        b"changed",
    )
    .unwrap();
    drop(held);
    discard(&f, operation.clone()).await.unwrap();
    assert_eq!(state(&f, &operation), Some(journal::State::Complete));
}

#[tokio::test]
async fn partial_filesystem_removal_resumes_without_reconstructing_deleted_inventory() {
    let (f, operation) = prepared().await;
    let copies = open(&f.store).unwrap();
    let held = copies.restore(&operation.identity()).unwrap();
    let changes = held.changes().unwrap();
    begin(&f, &operation);
    held.prepare_discard(&changes).unwrap();
    std::fs::remove_dir_all(
        f.store
            .agent_workspace_directory()
            .join(operation.binding.lease_id.to_string())
            .join("base"),
    )
    .unwrap();
    discard(&f, operation.clone()).await.unwrap();
    assert_eq!(state(&f, &operation), Some(journal::State::Complete));
}

#[tokio::test]
async fn database_completion_without_file_receipt_never_authorizes_new_deletion() {
    let (f, operation) = prepared().await;
    begin(&f, &operation);
    {
        let mut db = f.store.connection().unwrap();
        let mut tx = db.transaction().unwrap();
        journal::complete_in(&mut tx, &operation).unwrap();
        tx.commit().unwrap();
    }
    assert!(matches!(
        discard(&f, operation.clone()).await,
        Err(Error::RecoveryRequired)
    ));
    let held = open(&f.store)
        .unwrap()
        .restore(&operation.identity())
        .unwrap();
    assert_eq!(
        std::fs::read(held.working_directory().unwrap().join("main.tex")).unwrap(),
        b"changed"
    );
}
