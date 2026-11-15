use super::*;
use crate::agent::repository::SqliteConversationStore;
use std::sync::Mutex;

#[tokio::test]
async fn create_retry_preserves_history_and_read_page_matches_wire_contract() {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 67);
    let connection = Arc::new(Mutex::new(db));
    let store =
        Arc::new(SqliteConversationStore::new(connection.clone(), "Session".into()).unwrap());
    let service = AgentSessionService::new(store);
    let events: Vec<AgentEvent> = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/agent-events-v1.json"
    ))
    .unwrap();
    let id = events[0].conversation_id();
    let (first, retry) = tokio::join!(service.create_or_open(id), service.create_or_open(id));
    assert_eq!(first.unwrap().seq(), 0);
    assert_eq!(retry.unwrap().seq(), 0);
    service.commit(id, events.clone()).await.unwrap();
    assert_eq!(
        service.create_or_open(id).await.unwrap().seq(),
        events.len() as u64
    );
    let response = read_page(
        &service,
        HistoryRequest {
            snapshot_id: None,
            conversation_id: *id.as_uuid(),
            after_seq: 0,
            limit: 3,
        },
    )
    .await
    .unwrap();
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/agent-history-v1.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), expected);
    assert!(
        read_page(
            &service,
            HistoryRequest {
                snapshot_id: None,
                conversation_id: *id.as_uuid(),
                after_seq: 999,
                limit: 3
            }
        )
        .await
        .is_err()
    );
    assert!(
        read_page(
            &service,
            HistoryRequest {
                snapshot_id: None,
                conversation_id: *id.as_uuid(),
                after_seq: 0,
                limit: 0
            }
        )
        .await
        .is_err()
    );
    connection.lock().unwrap().execute(
        "update agent_items set payload_json = json_set(payload_json, '$.conversationId', ?1) where ordinal = 0",
        [Uuid::nil().to_string()],
    ).unwrap();
    assert!(matches!(
        read_page(
            &service,
            HistoryRequest {
                snapshot_id: None,
                conversation_id: *id.as_uuid(),
                after_seq: 0,
                limit: 3,
            }
        )
        .await,
        Err(SessionCommandError::Corrupt)
    ));
}

#[test]
fn wait_response_matches_shared_fixture_for_change_and_timeout() {
    let actual: Vec<_> = [Some(4), None]
        .into_iter()
        .map(|watermark| WaitResponse {
            schema_version: 1,
            conversation_id: Uuid::from_u128(1),
            after_seq: 3,
            watermark,
            events: None,
            resync_required: None,
        })
        .collect();
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/agent-history-wait-v1.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(actual).unwrap(), expected);
}
