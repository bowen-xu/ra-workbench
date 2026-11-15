//! Real workspace discard receipt followed by a refused SQL publication.
use super::*;
use crate::{agent::repository::SqliteConversationCatalog, data_interface::DeletionFailure};
use research_agent_core::conversation::{
    ArchiveConversation, ConversationCatalog, ConversationDeletionRequest,
    ConversationDeletionStore, ConversationId,
};

#[tokio::test]
async fn batch_deletion_defers_workspace_discard_until_committed_and_gc_survives_restart() {
    let f = fixture_with_objects();
    let review = changed(&f).await;
    let id = ConversationId::from_uuid(f.conversation);
    let catalog = SqliteConversationCatalog::from_app_store(&f.store);
    let current = catalog.read(id, None).await.unwrap();
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(current.durable_count),
        })
        .await
        .unwrap();
    let current = catalog.read(id, None).await.unwrap();
    let request = ConversationDeletionRequest {
        request_id: Uuid::new_v4(),
        conversation_id: id,
        binding: None,
        expected_title: current.title,
        expected_durable_count: current.durable_count,
        force: true,
    };
    let lease = f
        .store
        .agent_workspace_directory()
        .join(review.binding.lease_id.to_string());
    // A stale or corrupt queue entry must never authorize discarding a copy
    // while its conversation is still live and has no matching tombstone.
    let unrelated = Uuid::new_v4();
    f.store.connection().unwrap().execute(
        "insert into agent_deleted_workspace_gc(operation_id,conversation_id,turn_id,manuscript_id,lease_id) select ?1,conversation_id,turn_id,manuscript_id,lease_id from agent_execution_workspaces where conversation_id=?2",
        rusqlite::params![unrelated.to_string(), f.conversation.to_string()],
    ).unwrap();
    catalog.collect_deleted_workspaces().unwrap();
    assert!(lease.exists());
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_deleted_workspace_gc where operation_id=?1",
            [unrelated.to_string()],
        )
        .unwrap();
    let root = f.store.agent_workspace_directory().canonicalize().unwrap();
    let workspace = crate::agent::workspace::WorkspaceStore::open(&root, |_| Ok(())).unwrap();
    let identity = research_agent_core::workspace::WorkspaceIdentity {
        conversation_id: f.conversation,
        turn_id: review.binding.turn_id,
        environment_id: format!("manuscript:{}", f.manuscript),
        lease_id: review.binding.lease_id,
    };
    let owned = workspace.restore(&identity).unwrap();
    assert!(matches!(
        catalog.prepare_batch_deletion(request.clone()).await,
        Err(
            research_agent_core::conversation::ConversationDeletionError::Catalog(
                research_agent_core::conversation::CatalogError::Busy
            )
        )
    ));
    assert!(lease.exists());
    assert!(
        catalog
            .pending_deleted_workspace_operations()
            .unwrap()
            .is_empty()
    );
    drop(owned);
    let first = catalog
        .prepare_batch_deletion(request.clone())
        .await
        .unwrap();
    assert!(lease.exists());
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: false,
            expected_watermark: None,
        })
        .await
        .unwrap();
    assert!(catalog.publish_deletion_batch(vec![first]).await.is_err());
    assert!(lease.exists());
    assert!(
        catalog
            .pending_deleted_workspace_operations()
            .unwrap()
            .is_empty()
    );
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(request.expected_durable_count),
        })
        .await
        .unwrap();
    let prepared = catalog
        .prepare_batch_deletion(request.clone())
        .await
        .unwrap();
    catalog
        .publish_deletion_batch(vec![prepared])
        .await
        .unwrap();
    assert!(lease.exists());
    assert!(
        catalog
            .pending_deleted_workspace_operations()
            .unwrap()
            .contains(&request.request_id)
    );
    // Recreating the application-owned service runs the same startup collector.
    let _reopened = crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while lease.exists()
            || !catalog
                .pending_deleted_workspace_operations()
                .unwrap()
                .is_empty()
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!lease.exists());
    assert!(
        catalog
            .pending_deleted_workspace_operations()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn batch_deletion_refuses_a_replacement_behind_a_completed_discard_receipt() {
    let f = fixture_with_objects();
    let review = changed(&f).await;
    let id = ConversationId::from_uuid(f.conversation);
    let catalog = SqliteConversationCatalog::from_app_store(&f.store);
    let current = catalog.read(id, None).await.unwrap();
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(current.durable_count),
        })
        .await
        .unwrap();
    let current = catalog.read(id, None).await.unwrap();
    let request = ConversationDeletionRequest {
        request_id: Uuid::new_v4(),
        conversation_id: id,
        binding: None,
        expected_title: current.title,
        expected_durable_count: current.durable_count,
        force: true,
    };
    let root = f.store.agent_workspace_directory().canonicalize().unwrap();
    let workspace = crate::agent::workspace::WorkspaceStore::open(&root, |_| Ok(())).unwrap();
    let identity = research_agent_core::workspace::WorkspaceIdentity {
        conversation_id: f.conversation,
        turn_id: review.binding.turn_id,
        environment_id: format!("manuscript:{}", f.manuscript),
        lease_id: review.binding.lease_id,
    };
    workspace.discard(&identity).unwrap();
    let replacement = root.join(review.binding.lease_id.to_string());
    std::fs::create_dir(&replacement).unwrap();
    assert!(matches!(
        catalog.prepare_batch_deletion(request).await,
        Err(
            research_agent_core::conversation::ConversationDeletionError::Catalog(
                research_agent_core::conversation::CatalogError::Conflict
            )
        )
    ));
    assert!(replacement.exists());
    assert!(catalog.read(id, None).await.is_ok());
}

#[tokio::test]
async fn deletion_after_real_workspace_cleanup_keeps_partial_until_original_metadata_retry() {
    let f = fixture_with_objects();
    let review = changed(&f).await;
    let id = ConversationId::from_uuid(f.conversation);
    let catalog = SqliteConversationCatalog::from_app_store(&f.store);
    let current = catalog.read(id, None).await.unwrap();
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(current.durable_count),
        })
        .await
        .unwrap();
    let current = catalog.read(id, None).await.unwrap();
    let request = ConversationDeletionRequest {
        request_id: Uuid::new_v4(),
        conversation_id: id,
        binding: None,
        expected_title: current.title,
        expected_durable_count: current.durable_count,
        force: true,
    };
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    let original = std::fs::read(&source).unwrap();
    let lease = f
        .store
        .agent_workspace_directory()
        .join(review.binding.lease_id.to_string());
    assert!(lease.exists());
    let prepared = catalog.prepare_deletion(request.clone()).await.unwrap();
    assert!(!lease.exists());
    assert!(
        f.store
            .agent_workspace_directory()
            .join(format!("discarded-{}.json", review.binding.lease_id))
            .exists()
    );
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: false,
            expected_watermark: None,
        })
        .await
        .unwrap();
    let db = f.store.shared_agent_connection();
    let Err(DeletionFailure::CleanupComplete(cause)) = db.delete_conversation(&prepared) else {
        panic!("external cleanup must not become an ordinary SQL refusal");
    };
    assert_eq!(
        cause.safe_cause,
        "delete_not_archived_after_workspace_cleanup"
    );
    assert_eq!(cause.operation_id, request.request_id);
    assert!(catalog.read(id, None).await.is_ok());
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from agent_conversation_deletions where request_id=?1",
                [request.request_id.to_string()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(request.expected_durable_count),
        })
        .await
        .unwrap();
    assert_eq!(
        catalog.delete(request.clone()).await.unwrap().request_id,
        request.request_id
    );
    assert!(!lease.exists());
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[tokio::test]
async fn deletion_keeps_partial_cleanup_when_a_later_real_lease_is_still_owned() {
    let f = fixture_with_objects();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let mut owned = None;
    let mut first = None;
    for n in [1001, 1002] {
        let prepared = service
            .prepare(&f.models, request(&f, Uuid::from_u128(n)))
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
        if n == 1001 {
            first = Some(copy.lease.working_directory().unwrap().to_path_buf());
            drop(copy);
        } else {
            owned = Some(copy);
        }
    }
    let id = ConversationId::from_uuid(f.conversation);
    let catalog = SqliteConversationCatalog::from_app_store(&f.store);
    let current = catalog.read(id, None).await.unwrap();
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(current.durable_count),
        })
        .await
        .unwrap();
    let request = ConversationDeletionRequest {
        request_id: Uuid::new_v4(),
        conversation_id: id,
        binding: None,
        expected_title: current.title,
        expected_durable_count: current.durable_count,
        force: true,
    };
    let error = catalog.delete(request.clone()).await.unwrap_err();
    let wire =
        serde_json::to_value(crate::agent::deletion_commands::DeleteError::from(error)).unwrap();
    assert_eq!(wire["reason"]["code"], "DATA_OPERATION_PARTIAL");
    assert_eq!(
        wire["reason"]["reason"]["safeCause"],
        "workspace_discard_partial_busy"
    );
    assert!(!first.unwrap().exists());
    assert!(
        owned
            .as_ref()
            .unwrap()
            .lease
            .working_directory()
            .unwrap()
            .exists()
    );
    assert!(catalog.read(id, None).await.is_ok());
    drop(owned);
    assert_eq!(
        catalog.delete(request.clone()).await.unwrap().request_id,
        request.request_id
    );
}

#[tokio::test]
async fn deletion_epoch_never_touches_a_real_workspace_in_the_wrong_environment() {
    let f = fixture_with_objects();
    let review = changed(&f).await;
    let id = ConversationId::from_uuid(f.conversation);
    let catalog = SqliteConversationCatalog::from_app_store(&f.store);
    let current = catalog.read(id, None).await.unwrap();
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id,
            archived: true,
            expected_watermark: Some(current.durable_count),
        })
        .await
        .unwrap();
    let request = ConversationDeletionRequest {
        request_id: Uuid::new_v4(),
        conversation_id: id,
        binding: None,
        expected_title: current.title,
        expected_durable_count: current.durable_count,
        force: true,
    };
    let lease = f
        .store
        .agent_workspace_directory()
        .join(review.binding.lease_id.to_string());
    let receipt = f
        .store
        .agent_workspace_directory()
        .join(format!("discarded-{}.json", review.binding.lease_id));
    let changes = f.store.connection().unwrap().total_changes();
    assert!(lease.exists());
    assert!(!receipt.exists());
    let error = catalog
        .delete_in_epoch(request.clone(), Uuid::new_v4())
        .await
        .unwrap_err();
    let wire =
        serde_json::to_value(crate::agent::deletion_commands::DeleteError::from(error)).unwrap();
    assert_eq!(wire["reason"]["reason"]["phase"], "delete-epoch");
    assert_eq!(
        wire["reason"]["reason"]["operationId"],
        request.request_id.to_string()
    );
    assert!(lease.exists());
    assert!(!receipt.exists());
    assert_eq!(f.store.connection().unwrap().total_changes(), changes);
    assert!(catalog.read(id, None).await.is_ok());
}
