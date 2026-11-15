use super::*;
use research_agent_core::{
    conversation::{ConversationSearch, ConversationSearchQuery},
    protocol::{AgentEvent, EventPayload as P, ItemKind},
};
use std::time::Instant;

#[tokio::test]
#[ignore = "Local 10,000-message search/backfill acceptance; run explicitly to record timing"]
async fn ten_thousand_visible_messages_search_without_replaying_events() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 84);
    let owner = ConversationId::from_uuid(Uuid::from_u128(1));
    let turn_id = Uuid::from_u128(2);
    crate::agent::repository::create_conversation(&mut db, owner, "Search performance fixture")
        .unwrap();
    let mut count = 0;
    for batch in 0..100 {
        let mut events = Vec::new();
        for n in batch * 100..(batch + 1) * 100 {
            let item_id = Uuid::from_u128(100 + n);
            for payload in [
                P::ItemStarted {
                    turn_id,
                    item_id,
                    kind: ItemKind::AssistantMessage,
                },
                P::TextDelta {
                    turn_id,
                    item_id,
                    text: format!("研究资料 Evidence message {n:05} — searchable text."),
                },
                P::ItemCompleted { turn_id, item_id },
            ] {
                events.push(
                    AgentEvent::new(
                        owner,
                        count + events.len() as u64 + 1,
                        0,
                        Uuid::from_u128(3),
                        payload,
                    )
                    .unwrap(),
                );
            }
        }
        count =
            crate::agent::repository::append_batch(&mut db, owner, Uuid::new_v4(), count, &events)
                .unwrap();
    }
    let start = Instant::now();
    db.execute_batch(include_str!(
        "../../../../../migrations/0085_agent_search_messages.sql"
    ))
    .unwrap();
    eprintln!(
        "search backfill: {} ms for 30,000 events / 10,000 messages",
        start.elapsed().as_millis()
    );
    let count: i64 = db
        .query_row("select count(*) from agent_search_messages", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 10_000);
    db.execute(
        "insert into schema_migrations(version,applied_at) values(85,'now')",
        [],
    )
    .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let catalog = SqliteConversationCatalog::new(Arc::new(Mutex::new(db)));
    let mut elapsed = Vec::new();
    for _ in 0..20 {
        let start = Instant::now();
        let page = catalog
            .search(ConversationSearchQuery {
                term: "message 09999".into(),
                scope: CatalogQuery {
                    internal: false,
                    binding: None,
                    archived: false,
                    limit: 10,
                    before: None,
                },
            })
            .await
            .unwrap();
        elapsed.push(start.elapsed());
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].item_id, Some(Uuid::from_u128(100 + 9999)));
    }
    elapsed.sort();
    eprintln!(
        "visible-message search P95: {:?} (20 queries, one 10,000-message conversation)",
        elapsed[18]
    );
    assert!(elapsed[18].as_millis() <= 250);
}
