//! Production store/service/subscription chain over an isolated logged database.
use super::Database;
use crate::agent::{repository::SqliteConversationStore, service::AgentSessionService};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{
        AgentEvent, ApplyResult, ConversationProjection, EventPayload, ItemKind, ModelIdentity,
        TaskKind, TurnPhase,
    },
};
use rusqlite::{Connection, OpenFlags};
use std::sync::Arc;
use uuid::Uuid;

fn sql_watermarks(observer: &Connection, id: ConversationId) -> (u64, u64) {
    observer
        .query_row(
            "select c.durable_count,s.materialized_position from agent_conversations c
         cross join data_runtime_state s where c.id=?1 and s.singleton=1",
            [id.as_uuid().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

async fn apply_accepted(
    service: &AgentSessionService,
    id: ConversationId,
    projection: &mut ConversationProjection,
    received: &mut Vec<AgentEvent>,
) {
    let after = projection.seq();
    let update = service
        .wait_for_accepted_history(id, after, 0)
        .await
        .unwrap();
    assert!(
        !update.resync_required,
        "an active bounded subscriber must keep its sequence"
    );
    assert!(!update.events.is_empty());
    for event in update.events {
        assert_eq!(projection.apply(&event).unwrap(), ApplyResult::Applied);
        received.push(event);
    }
    assert_eq!(update.watermark, Some(projection.seq()));
}

#[tokio::test]
async fn accepted_service_feed_does_not_materialize_each_streamed_fragment() {
    const FRAGMENTS: u64 = 128;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("service.sqlite3");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = Database::open(sql, &directory.path().join("data-log")).unwrap();
    let observer = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let store = Arc::new(
        SqliteConversationStore::<AgentEvent>::new(database, "service fixture".into()).unwrap(),
    );
    let service = AgentSessionService::new(store);
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let request = Uuid::new_v4();
    let item = Uuid::new_v4();
    service.create(id).await.unwrap();
    service
        .commit_payload(
            id,
            request,
            EventPayload::TurnStarted {
                task_kind: TaskKind::Regular,
                turn_id: turn,
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            },
        )
        .await
        .unwrap();
    let boundary = sql_watermarks(&observer, id);
    assert_eq!(boundary.0, 1);
    let mut projection = ConversationProjection::new(id);
    let mut received = Vec::new();
    apply_accepted(&service, id, &mut projection, &mut received).await;
    service
        .commit_payloads(
            id,
            request,
            vec![
                EventPayload::PhaseChanged {
                    turn_id: turn,
                    phase: TurnPhase::RunningModel,
                },
                EventPayload::ItemStarted {
                    turn_id: turn,
                    item_id: item,
                    kind: ItemKind::AssistantMessage,
                },
            ],
        )
        .await
        .unwrap();
    let mut expected_body = String::new();
    for index in 0..FRAGMENTS {
        let text = format!("片段{index}🙂\0");
        expected_body.push_str(&text);
        service
            .commit_payload(
                id,
                request,
                EventPayload::TextDelta {
                    turn_id: turn,
                    item_id: item,
                    text,
                },
            )
            .await
            .unwrap();
        apply_accepted(&service, id, &mut projection, &mut received).await;
        // The observer bypasses the facade's query barrier on purpose. Every
        // subscriber update above must leave the SQL prefix at TurnStarted.
        assert_eq!(sql_watermarks(&observer, id), boundary);
    }
    assert_eq!(projection.seq(), FRAGMENTS + 3);
    assert_eq!(
        projection.turn(turn).unwrap().items[&item].text,
        expected_body
    );

    // One ordinary history query is the materialization boundary for the tail.
    let page = service.read_since(id, 0, 500).await.unwrap();
    assert_eq!(page.items, received);
    assert_eq!(page.durable_count, projection.seq());
    assert!(page.next_offset.is_none());
    let materialized = sql_watermarks(&observer, id);
    assert_eq!(materialized.0, projection.seq());
    assert!(materialized.1 > boundary.1);
    assert_eq!(
        observer
            .query_row(
                "select body from agent_messages where conversation_id=?1 and item_id=?2",
                [id.as_uuid().to_string(), item.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        expected_body
    );
    let mut replay = ConversationProjection::new(id);
    for event in page.items {
        replay.apply(&event).unwrap();
    }
    assert_eq!(replay, projection);
}
