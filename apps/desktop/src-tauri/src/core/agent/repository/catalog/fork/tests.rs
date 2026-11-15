use super::*;
#[path = "tests/compaction.rs"]
mod compaction;
#[path = "tests/origin.rs"]
mod origin;
#[path = "tests/reading_note.rs"]
mod reading_note;
use crate::agent::repository::{append_batch, create_conversation};
use research_agent_core::protocol::{
    AgentEvent, EventPayload as P, ItemKind, ModelIdentity, TurnOutcome,
};

fn id(n: u128) -> ConversationId {
    ConversationId::from_uuid(Uuid::from_u128(n))
}
fn request(child: u128) -> ConversationForkRequest {
    ConversationForkRequest {
        child_id: id(child),
        parent_id: id(1),
        through_turn_id: Uuid::from_u128(10),
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "note".into(),
        },
    }
}
fn seed(db: &mut Connection) {
    crate::storage_migration_tests::migrate_through(db, 144);
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at) values ('note','Note','now','now')",
        [],
    )
    .unwrap();
    create_conversation(db, id(1), "研究讨论").unwrap();
    db.execute(
        "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','note')",
        [id(1).as_uuid().to_string()],
    )
    .unwrap();
    let turn = Uuid::from_u128(10);
    let item = Uuid::from_u128(11);
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "fixture".into(),
        revision: None,
    };
    let events = [
        P::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: turn,
            context_snapshot_id: Uuid::from_u128(12),
            model: model.clone(),
        },
        P::ItemStarted {
            turn_id: turn,
            item_id: item,
            kind: ItemKind::UserMessage,
        },
        P::TextDelta {
            turn_id: turn,
            item_id: item,
            text: "历史标记".into(),
        },
        P::ItemCompleted {
            turn_id: turn,
            item_id: item,
        },
        P::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        },
        P::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: Uuid::from_u128(20),
            context_snapshot_id: Uuid::from_u128(21),
            model,
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, payload)| {
        AgentEvent::new(id(1), index as u64 + 1, 123, Uuid::from_u128(30), payload).unwrap()
    })
    .collect::<Vec<_>>();
    append_batch(db, id(1), Uuid::new_v4(), 0, &events).unwrap();
}
fn setup() -> (Arc<Mutex<Connection>>, SqliteConversationCatalog) {
    let mut db = Connection::open_in_memory().unwrap();
    seed(&mut db);
    let connection = Arc::new(Mutex::new(db));
    (
        connection.clone(),
        SqliteConversationCatalog::new(connection),
    )
}

#[tokio::test]
async fn materializes_completed_prefix_atomically_and_replays_without_resetting_child() {
    let (db, catalog) = setup();
    let (one, two) = tokio::join!(
        catalog.fork_legacy_for_tests(request(2)),
        catalog.fork_legacy_for_tests(request(2))
    );
    assert_eq!(one.as_ref().unwrap(), two.as_ref().unwrap());
    let fork = one.unwrap();
    assert_eq!(fork.parent_end_seq, 5);
    assert_eq!(fork.conversation.durable_count, 5);
    let exported = catalog.export_snapshot(id(2)).await.unwrap();
    assert_eq!(exported.turns.len(), 1);
    assert_eq!(exported.turns[0].items[0].text, "历史标记");
    assert_eq!(exported.turns[0].context_snapshot_id, Uuid::from_u128(12));
    assert_eq!(exported.turns[0].outcome, Some(TurnOutcome::Completed));
    assert_eq!(catalog.read(id(1), None).await.unwrap().durable_count, 6);
    catalog
        .rename_legacy_for_tests(id(2), fork.conversation.title, "Renamed".into())
        .await
        .unwrap();
    catalog.archive_legacy_for_tests(id(2), true).await.unwrap();
    let changes = db.lock().unwrap().total_changes();
    let replayed = catalog.fork_legacy_for_tests(request(2)).await.unwrap();
    assert_eq!(replayed.conversation.title, "Renamed");
    assert!(replayed.conversation.archived);
    assert_eq!(changes, db.lock().unwrap().total_changes());
    let mut changed = request(2);
    changed.through_turn_id = Uuid::from_u128(20);
    assert_eq!(
        catalog.fork_legacy_for_tests(changed).await.unwrap_err(),
        CatalogError::Conflict
    );
    let guard = db.lock().unwrap();
    // No copied journal batch identity, execution admissions or queue entries.
    assert_eq!(
        guard
            .query_row(
                "select count(*) from agent_journal_batches where conversation_id=?1",
                [id(2).as_uuid().to_string()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        guard
            .query_row(
                "select body from agent_search_messages where conversation_id=?1",
                [id(2).as_uuid().to_string()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "历史标记"
    );
}

#[tokio::test]
async fn rejects_active_or_missing_boundaries_and_rolls_back_partial_publication() {
    let (db, catalog) = setup();
    let mut active = request(2);
    active.through_turn_id = Uuid::from_u128(20);
    assert_eq!(
        catalog.fork_legacy_for_tests(active).await.unwrap_err(),
        CatalogError::Busy
    );
    let mut missing = request(2);
    missing.through_turn_id = Uuid::from_u128(999);
    assert_eq!(
        catalog.fork_legacy_for_tests(missing).await.unwrap_err(),
        CatalogError::NotFound
    );
    let mut wrong_host = request(2);
    wrong_host.binding.resource_id = "other".into();
    assert_eq!(
        catalog.fork_legacy_for_tests(wrong_host).await.unwrap_err(),
        CatalogError::NotFound
    );
    assert_eq!(
        catalog.fork_legacy_for_tests(request(1)).await.unwrap_err(),
        CatalogError::InvalidInput
    );
    db.lock().unwrap().execute_batch("create trigger reject_lineage before insert on agent_conversation_forks begin select raise(abort, 'injected'); end;").unwrap();
    assert!(catalog.fork_legacy_for_tests(request(2)).await.is_err());
    assert_eq!(
        catalog.read(id(2), None).await.unwrap_err(),
        CatalogError::NotFound
    );
    {
        let guard = db.lock().unwrap();
        assert_eq!(
            guard
                .query_row(
                    "select count(*) from agent_search_messages where conversation_id=?1",
                    [id(2).as_uuid().to_string()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        guard.execute_batch("drop trigger reject_lineage").unwrap();
    }
    assert!(catalog.fork_legacy_for_tests(request(2)).await.is_ok());
}

#[tokio::test]
async fn retains_parent_provenance_and_can_fork_a_branch_after_database_reopen() {
    let file = tempfile::NamedTempFile::new().unwrap();
    {
        let mut db = Connection::open(file.path()).unwrap();
        seed(&mut db);
    }
    let db = Arc::new(Mutex::new(Connection::open(file.path()).unwrap()));
    db.lock()
        .unwrap()
        .execute_batch("pragma foreign_keys=on")
        .unwrap();
    let catalog = SqliteConversationCatalog::new(db.clone());
    catalog.fork_legacy_for_tests(request(2)).await.unwrap();
    let mut branch = request(3);
    branch.parent_id = id(2);
    catalog.fork_legacy_for_tests(branch).await.unwrap();
    let guard = db.lock().unwrap();
    assert!(
        guard
            .execute(
                "delete from agent_conversations where id=?1",
                [id(1).as_uuid().to_string()]
            )
            .is_err()
    );
    assert!(
        guard
            .execute(
                "delete from agent_conversations where id=?1",
                [id(2).as_uuid().to_string()]
            )
            .is_err()
    );
    guard
        .execute(
            "delete from agent_conversations where id=?1",
            [id(3).as_uuid().to_string()],
        )
        .unwrap();
    assert_eq!(
        guard
            .query_row("select count(*) from agent_conversation_forks", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
    assert_eq!(
        guard
            .query_row("select count(*) from agent_conversations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
