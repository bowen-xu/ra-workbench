use super::*;
use crate::{agent::repository::SqliteConversationStore, data_interface::Database};
use research_agent_core::protocol::{EventPayload, ItemKind, ModelIdentity, TaskKind, TurnPhase};
use rusqlite::Connection;

async fn fixture() -> (
    tempfile::TempDir,
    AgentSessionService,
    Connection,
    ConversationId,
    Uuid,
    Uuid,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot.sqlite3");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = Database::open(sql, &directory.path().join("data-log")).unwrap();
    let store = Arc::new(
        SqliteConversationStore::<AgentEvent>::new(database.clone(), "snapshot".into()).unwrap(),
    );
    let service = AgentSessionService::new_with_compaction_db(store, Some(database));
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    service.create(id).await.unwrap();
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
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
    service
        .commit_payloads(
            id,
            Uuid::new_v4(),
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
                EventPayload::TextDelta {
                    turn_id: turn,
                    item_id: item,
                    text: "before🙂\0".into(),
                },
            ],
        )
        .await
        .unwrap();
    (
        directory,
        service,
        Connection::open(path).unwrap(),
        id,
        turn,
        item,
    )
}

#[tokio::test]
async fn frozen_pages_keep_one_watermark_and_body_version_without_flushing_the_live_tail() {
    let (_directory, service, observer, id, turn, item) = fixture().await;
    let first = service
        .read_frozen_history(HistoryQuery::new(id, 0, 3).unwrap(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.page.durable_count, 4);
    assert_eq!(first.page.items.len(), 3);
    service
        .commit_payload(
            id,
            Uuid::new_v4(),
            EventPayload::TextDelta {
                turn_id: turn,
                item_id: item,
                text: " tail".into(),
            },
        )
        .await
        .unwrap();
    let second = service
        .read_frozen_history(
            HistoryQuery::new(id, 3, 3).unwrap(),
            Some(first.snapshot_id),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.page.durable_count, 4);
    assert!(second.page.next_offset.is_none());
    assert!(
        matches!(second.page.items[0].payload(),EventPayload::TextDelta{text,..} if text=="before🙂\0")
    );
    assert_eq!(
        observer
            .query_row("select durable_count from agent_conversations", [], |r| r
                .get::<_, u64>(
                0
            ))
            .unwrap(),
        4
    );
    let live = service
        .read_frozen_history(HistoryQuery::new(id, 3, 3).unwrap(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(live.page.durable_count, 5);
    assert_eq!(live.page.items.len(), 2);
    // Deliberately simulate a future SQL projection rewrite. A snapshot token
    // freezes complete bodies, rather than relying on append-only assumptions.
    observer.execute("update agent_journal_records set payload_json=json_set(payload_json,'$.payload.text','later view') where ordinal=3",[]).unwrap();
    let stable = service
        .read_frozen_history(
            HistoryQuery::new(id, 3, 3).unwrap(),
            Some(first.snapshot_id),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stable.page.items, second.page.items);
    let latest = service
        .read_frozen_history(HistoryQuery::new(id, 3, 3).unwrap(), None)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(latest.page.items[0].payload(),EventPayload::TextDelta{text,..} if text=="later view")
    );
}

#[tokio::test]
async fn token_cannot_cross_conversations_and_invalidated_or_expired_tokens_fail_explicitly() {
    let (_directory, service, _observer, id, _, _) = fixture().await;
    let first = service
        .read_frozen_history(HistoryQuery::new(id, 0, 1).unwrap(), None)
        .await
        .unwrap()
        .unwrap();
    let foreign = ConversationId::from_uuid(Uuid::new_v4());
    assert!(matches!(
        service
            .read_frozen_history(
                HistoryQuery::new(foreign, 1, 1).unwrap(),
                Some(first.snapshot_id)
            )
            .await,
        Err(HistorySnapshotError::InvalidCursor)
    ));
    service
        .history_snapshots
        .lock()
        .unwrap()
        .remove_expired(Instant::now() + TTL + Duration::from_secs(1));
    assert!(matches!(
        service
            .read_frozen_history(
                HistoryQuery::new(id, 1, 1).unwrap(),
                Some(first.snapshot_id)
            )
            .await,
        Err(HistorySnapshotError::Expired)
    ));
    let next = service
        .read_frozen_history(HistoryQuery::new(id, 0, 1).unwrap(), None)
        .await
        .unwrap()
        .unwrap();
    service.invalidate_history_snapshots(Some(id));
    assert!(matches!(
        service
            .read_frozen_history(HistoryQuery::new(id, 1, 1).unwrap(), Some(next.snapshot_id))
            .await,
        Err(HistorySnapshotError::Expired)
    ));
}
