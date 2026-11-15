use super::*;
use crate::agent::repository::{append_batch, create_conversation};
use research_agent_core::{
    conversation::{ConversationSearch, ConversationSearchQuery},
    protocol::{AgentEvent, EventPayload as P, ItemKind, ModelIdentity},
};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn owner(n: u128) -> ConversationId {
    ConversationId::from_uuid(id(n))
}
fn database(version: i64) -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, version);
    db
}
fn write(db: &mut Connection, n: u128, after: u64, payloads: Vec<P>) -> u64 {
    let events: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(i, payload)| {
            AgentEvent::new(owner(n), after + i as u64 + 1, 0, id(900), payload).unwrap()
        })
        .collect();
    let batch = Uuid::new_v4();
    let count = append_batch(db, owner(n), batch, after, &events).unwrap();
    assert_eq!(
        append_batch(db, owner(n), batch, after, &events).unwrap(),
        count
    );
    count
}
fn seed(db: &mut Connection, n: u128) -> u64 {
    create_conversation(db, owner(n), "Conversation").unwrap();
    write(
        db,
        n,
        0,
        vec![
            P::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: id(2),
                context_snapshot_id: id(3),
                model: ModelIdentity {
                    provider_id: "private-provider-needle".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            },
            P::ItemStarted {
                turn_id: id(2),
                item_id: id(10),
                kind: ItemKind::UserMessage,
            },
            P::TextDelta {
                turn_id: id(2),
                item_id: id(10),
                text: "第一段🙂跨片".into(),
            },
            P::TextDelta {
                turn_id: id(2),
                item_id: id(10),
                text: "段 Evidence 100%_ [x]".into(),
            },
            P::ItemCompleted {
                turn_id: id(2),
                item_id: id(10),
            },
            P::ItemStarted {
                turn_id: id(2),
                item_id: id(11),
                kind: ItemKind::ToolCall {
                    call_id: id(12),
                    name: "tool-only-needle".into(),
                    provider_call_id: None,
                    namespace: None,
                },
            },
            P::TextDelta {
                turn_id: id(2),
                item_id: id(11),
                text: "private-argument-needle".into(),
            },
            P::ItemCompleted {
                turn_id: id(2),
                item_id: id(11),
            },
            P::ItemStarted {
                turn_id: id(2),
                item_id: id(13),
                kind: ItemKind::ToolResult {
                    call_id: id(12),
                    is_error: None,
                },
            },
            P::TextDelta {
                turn_id: id(2),
                item_id: id(13),
                text: "private-result-needle".into(),
            },
            P::ItemCompleted {
                turn_id: id(2),
                item_id: id(13),
            },
            P::ItemStarted {
                turn_id: id(2),
                item_id: id(14),
                kind: ItemKind::AssistantMessage,
            },
            P::TextDelta {
                turn_id: id(2),
                item_id: id(14),
                text: "Saved partial response".into(),
            },
        ],
    )
}
fn query(term: &str) -> ConversationSearchQuery {
    ConversationSearchQuery {
        term: term.into(),
        scope: CatalogQuery {
            internal: false,
            binding: None,
            archived: false,
            limit: 10,
            before: None,
        },
    }
}
fn rows(db: &Connection) -> Vec<(String, String, bool, i64)> {
    db.prepare("select item_id, body, completed, last_ordinal from agent_search_messages order by conversation_id, start_ordinal")
        .unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().map(Result::unwrap).collect()
}

#[tokio::test]
async fn projection_survives_database_reopen_without_runtime_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-search.sqlite");
    {
        let mut db = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        seed(&mut db, 1);
    }
    let reopened = Connection::open(&path).unwrap();
    let changes = reopened.total_changes();
    let db = Arc::new(Mutex::new(reopened));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let result = catalog.search(query("partial")).await.unwrap();
    assert_eq!(result.hits[0].item_id, Some(id(14)));
    assert!(!result.hits[0].completed);
    assert_eq!(db.lock().unwrap().total_changes(), changes);
}

#[tokio::test]
async fn visible_message_search_spans_deltas_excludes_internals_and_handles_literal_unicode() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    seed(&mut db, 1);
    let db = Arc::new(Mutex::new(db));
    let catalog = SqliteConversationCatalog::new(db.clone());
    for term in ["跨片段", "eVIDENCE", "100%_", "[x]", "🙂"] {
        let page = catalog.search(query(term)).await.unwrap();
        assert_eq!(page.hits.len(), 1, "{term}");
        assert_eq!(page.hits[0].item_id, Some(id(10)));
        assert_eq!(page.hits[0].start_seq, Some(2));
        assert!(page.hits[0].completed);
        assert!(page.next.is_none());
    }
    for term in [
        "private-provider-needle",
        "tool-only-needle",
        "private-argument-needle",
        "private-result-needle",
        "%missing",
        "' OR 1=1 --",
    ] {
        assert!(
            catalog.search(query(term)).await.unwrap().hits.is_empty(),
            "{term}"
        );
    }
    let partial = catalog.search(query("partial")).await.unwrap();
    assert!(!partial.hits[0].completed);
    assert_eq!(partial.hits[0].item_id, Some(id(14)));
    assert_eq!(
        catalog.search(query("Conversation")).await.unwrap().hits[0].item_id,
        None
    );
    for term in ["", "  ", "\0", &"a".repeat(201)] {
        assert_eq!(
            catalog.search(query(term)).await.unwrap_err(),
            CatalogError::InvalidInput
        );
    }
}

#[test]
fn old_history_backfill_matches_live_projection_and_rollbacks_do_not_leak() {
    let mut old = database(84);
    let after = seed(&mut old, 1);
    let mut live = database(85);
    seed(&mut live, 1);
    old.execute_batch(include_str!(
        "../../../../../migrations/0085_agent_search_messages.sql"
    ))
    .unwrap();
    assert_eq!(rows(&old), rows(&live));
    let original = rows(&live);
    let event = AgentEvent::new(
        owner(1),
        after + 1,
        0,
        id(900),
        P::TextDelta {
            turn_id: id(2),
            item_id: id(14),
            text: " uncommitted".into(),
        },
    )
    .unwrap();
    {
        let tx = live.transaction().unwrap();
        tx.execute(
            "insert into agent_items values(?1,?2,1,?3)",
            params![
                owner(1).as_uuid().to_string(),
                after,
                serde_json::to_string(&event).unwrap()
            ],
        )
        .unwrap();
        assert_ne!(rows(&tx), original);
        // Dropping the transaction must discard both the event and its search text.
    }
    assert_eq!(rows(&live), original);
    write(
        &mut live,
        1,
        after,
        vec![
            event.payload().clone(),
            P::ItemCompleted {
                turn_id: id(2),
                item_id: id(14),
            },
        ],
    );
    assert!(rows(&live)[1].1.ends_with(" uncommitted"));
    assert!(rows(&live)[1].2);
    live.execute(
        "delete from agent_conversations where id=?1",
        [owner(1).as_uuid().to_string()],
    )
    .unwrap();
    assert!(rows(&live).is_empty());
}

#[tokio::test]
async fn search_pages_respect_host_kind_archive_and_stable_identity() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    for n in 1..=4 {
        seed(&mut db, n);
    }
    db.execute_batch("update agent_conversations set created_at = '2026-09-10T00:00:00.000Z';")
        .unwrap();
    db.execute(
        "update agent_conversations set archived=1 where id=?1",
        [owner(3).as_uuid().to_string()],
    )
    .unwrap();
    for (n, kind) in [(1, "note"), (2, "note"), (3, "note"), (4, "paper")] {
        db.execute(
            "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,?2,'one')",
            params![owner(n).as_uuid().to_string(), kind],
        )
        .unwrap();
    }
    let catalog = SqliteConversationCatalog::new(Arc::new(Mutex::new(db)));
    let mut scoped = query("Evidence");
    scoped.scope.binding = Some(ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "one".into(),
    });
    scoped.scope.limit = 1;
    let first = catalog.search(scoped.clone()).await.unwrap();
    assert_eq!(first.hits[0].conversation.id, owner(2));
    scoped.scope.before = first.next;
    let last = catalog.search(scoped.clone()).await.unwrap();
    assert_eq!(last.hits[0].conversation.id, owner(1));
    assert!(last.next.is_none());
    scoped.scope.before = None;
    scoped.scope.archived = true;
    assert_eq!(
        catalog.search(scoped).await.unwrap().hits[0]
            .conversation
            .id,
        owner(3)
    );
}
