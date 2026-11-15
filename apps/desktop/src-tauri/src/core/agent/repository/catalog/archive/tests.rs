use super::*;
use crate::agent::repository::{JournalError, append_batch, create_conversation, read_history};
use research_agent_core::{
    conversation::HistoryQuery,
    protocol::{AgentEvent, EventPayload, ModelIdentity, TurnOutcome},
};

fn owner() -> ConversationId {
    ConversationId::from_uuid(Uuid::from_u128(1))
}
fn setup() -> (Arc<Mutex<Connection>>, SqliteConversationCatalog) {
    setup_connection(Connection::open_in_memory().unwrap())
}
fn setup_connection(mut db: Connection) -> (Arc<Mutex<Connection>>, SqliteConversationCatalog) {
    crate::storage_migration_tests::migrate_through(&mut db, 144);
    create_conversation(&mut db, owner(), "Archive test").unwrap();
    let db = Arc::new(Mutex::new(db));
    (db.clone(), SqliteConversationCatalog::new(db))
}

#[tokio::test]
async fn archive_survives_reopen_without_losing_resource_bindings() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("catalog.sqlite");
    let binding = ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "note".into(),
    };
    {
        let (db, catalog) = setup_connection(Connection::open(&path).unwrap());
        db.lock().unwrap().execute("insert into note_pages(id,title,created_at,updated_at) values ('note','Synthetic note','now','now')", []).unwrap();
        catalog
            .bind_legacy_for_tests(owner(), binding.clone())
            .await
            .unwrap();
        catalog
            .archive_legacy_for_tests(owner(), true)
            .await
            .unwrap();
    }
    let catalog =
        SqliteConversationCatalog::new(Arc::new(Mutex::new(Connection::open(path).unwrap())));
    let page = catalog
        .list(CatalogQuery {
            internal: false,
            binding: Some(binding.clone()),
            archived: true,
            limit: 10,
            before: None,
        })
        .await
        .unwrap();
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(page.conversations[0].id, owner());
    assert_eq!(page.conversations[0].durable_count, 0);
    catalog
        .archive_legacy_for_tests(owner(), false)
        .await
        .unwrap();
    let page = catalog
        .list(CatalogQuery {
            internal: false,
            binding: Some(binding),
            archived: false,
            limit: 10,
            before: None,
        })
        .await
        .unwrap();
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(page.conversations[0].durable_count, 0);
}
fn event(seq: u64, finish: bool) -> AgentEvent {
    AgentEvent::new(
        owner(),
        seq,
        0,
        Uuid::from_u128(4),
        if finish {
            EventPayload::TurnFinished {
                turn_id: Uuid::from_u128(2),
                outcome: TurnOutcome::Canceled,
            }
        } else {
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: Uuid::from_u128(2),
                context_snapshot_id: Uuid::from_u128(3),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            }
        },
    )
    .unwrap()
}

#[tokio::test]
async fn pending_live_writer_cannot_publish_after_archive_wins() {
    use crate::agent::repository::SqliteConversationStore;
    use research_agent_core::conversation::{
        ConversationStore, LiveConversation, StoreError, WriterId, WriterLease,
    };
    let (db, catalog) = setup();
    let store = Arc::new(SqliteConversationStore::<AgentEvent>::new(db, "Unused".into()).unwrap());
    let lease = WriterLease {
        conversation_id: owner(),
        writer_id: WriterId::from_uuid(Uuid::new_v4()),
    };
    let (live, _) = LiveConversation::resume(store.clone(), lease, 10)
        .await
        .unwrap();
    live.append(&[event(1, false)]).await.unwrap();
    catalog
        .archive_legacy_for_tests(owner(), true)
        .await
        .unwrap();
    assert_eq!(live.persist().await, Err(StoreError::WriterConflict));
    assert_eq!(
        store
            .history(HistoryQuery::new(owner(), 0, 10).unwrap())
            .await
            .unwrap()
            .durable_count,
        0
    );
    catalog
        .archive_legacy_for_tests(owner(), false)
        .await
        .unwrap();
    // Restore has not flushed the old pending input or started anything.
    assert_eq!(
        store
            .history(HistoryQuery::new(owner(), 0, 10).unwrap())
            .await
            .unwrap()
            .durable_count,
        0
    );
    live.discard();
}

#[tokio::test]
async fn archive_restore_preserve_history_and_exact_batch_replays() {
    let (db, catalog) = setup();
    let batch = Uuid::new_v4();
    let events = [event(1, false), event(2, true)];
    append_batch(&mut db.lock().unwrap(), owner(), batch, 0, &events).unwrap();
    let archived = catalog
        .archive_legacy_for_tests(owner(), true)
        .await
        .unwrap();
    assert!(archived.archived);
    assert_eq!(archived.durable_count, 2);
    assert_eq!(
        catalog
            .archive_legacy_for_tests(owner(), true)
            .await
            .unwrap(),
        archived
    );
    {
        let mut db = db.lock().unwrap();
        assert_eq!(
            append_batch(&mut db, owner(), batch, 0, &events).unwrap(),
            2
        );
        assert_eq!(
            append_batch(&mut db, owner(), Uuid::new_v4(), 2, &[event(3, false)]),
            Err(JournalError::Archived)
        );
        assert_eq!(
            read_history::<AgentEvent>(&mut db, HistoryQuery::new(owner(), 0, 10).unwrap())
                .unwrap()
                .items
                .len(),
            2
        );
    }
    let page = catalog
        .list(CatalogQuery {
            internal: false,
            binding: None,
            archived: true,
            limit: 10,
            before: None,
        })
        .await
        .unwrap();
    assert_eq!(page.conversations, [archived]);
    let restored = catalog
        .archive_legacy_for_tests(owner(), false)
        .await
        .unwrap();
    assert!(!restored.archived);
    assert_eq!(restored.durable_count, 2);
    assert_eq!(
        catalog
            .archive_legacy_for_tests(owner(), false)
            .await
            .unwrap(),
        restored
    );
    assert!(
        catalog
            .list(CatalogQuery {
                internal: false,
                binding: None,
                archived: true,
                limit: 10,
                before: None
            })
            .await
            .unwrap()
            .conversations
            .is_empty()
    );
}

#[tokio::test]
async fn active_turn_and_changed_watermark_prevent_archiving() {
    let (db, catalog) = setup();
    append_batch(
        &mut db.lock().unwrap(),
        owner(),
        Uuid::new_v4(),
        0,
        &[event(1, false)],
    )
    .unwrap();
    assert_eq!(
        catalog.archive_legacy_for_tests(owner(), true).await,
        Err(CatalogError::Busy)
    );
    // Archive observed an idle zero-watermark snapshot before this turn committed.
    assert_eq!(
        change_in(&mut db.lock().unwrap(), owner(), true, Some(0)),
        Err(CatalogError::Conflict)
    );
    assert!(
        !catalog
            .list(CatalogQuery {
                internal: false,
                binding: None,
                archived: false,
                limit: 10,
                before: None
            })
            .await
            .unwrap()
            .conversations[0]
            .archived
    );
}

#[tokio::test]
async fn invalid_or_corrupt_history_is_not_silently_archived() {
    let (db, catalog) = setup();
    assert_eq!(
        catalog
            .archive_legacy_for_tests(ConversationId::from_uuid(Uuid::nil()), true)
            .await,
        Err(CatalogError::InvalidInput)
    );
    assert_eq!(
        catalog
            .archive_legacy_for_tests(ConversationId::from_uuid(Uuid::from_u128(99)), false)
            .await,
        Err(CatalogError::NotFound)
    );
    append_batch(
        &mut db.lock().unwrap(),
        owner(),
        Uuid::new_v4(),
        0,
        &["not an Agent event"],
    )
    .unwrap();
    assert_eq!(
        catalog.archive_legacy_for_tests(owner(), true).await,
        Err(CatalogError::Corrupt)
    );
    // Restoring legacy archived metadata does not reinterpret or execute its history.
    db.lock()
        .unwrap()
        .execute("update agent_conversations set archived=1", [])
        .unwrap();
    assert!(
        !catalog
            .archive_legacy_for_tests(owner(), false)
            .await
            .unwrap()
            .archived
    );
}
