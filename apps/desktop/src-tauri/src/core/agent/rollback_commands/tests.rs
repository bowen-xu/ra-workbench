use super::*;
use crate::agent::repository::SqliteConversationStore;
use research_agent_core::{
    conversation::{BindingKind, ConversationCatalog},
    protocol::{EventPayload, ItemKind, ModelIdentity, TaskKind, TurnOutcome},
};
use rusqlite::Connection;
use std::sync::Mutex;
#[path = "tests/managed.rs"]
mod managed;
#[path = "tests/managed_status.rs"]
mod managed_status;
#[path = "tests/storage_error.rs"]
mod storage_error;

fn wire(id: Uuid, turn: Uuid, request: Uuid) -> serde_json::Value {
    serde_json::json!({"schemaVersion":1,"conversationId":id,"requestId":request,
        "binding":{"kind":"note","resourceId":"note-one"},"expectedWatermark":5,
        "expectedLastTurn":turn,"userTurns":1})
}
async fn setup() -> (
    Arc<Mutex<Connection>>,
    SqliteConversationCatalog,
    Arc<AgentSessionService>,
    Uuid,
    Uuid,
) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into note_pages(id,title,created_at,updated_at) values ('note-one','Note','now','now'),('note-two','Other','now','now')", []).unwrap();
    let db = Arc::new(Mutex::new(db));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let id = Uuid::new_v4();
    catalog
        .create_bound_legacy_for_tests(
            ConversationId::from_uuid(id),
            ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "note-one".into(),
            },
        )
        .await
        .unwrap();
    let service = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db.clone(), "Rollback".into()).unwrap(),
    )));
    service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = user_turn(&service, id).await;
    (db, catalog, service, id, turn)
}
async fn user_turn(service: &AgentSessionService, id: Uuid) -> Uuid {
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    service
        .commit_payloads(
            ConversationId::from_uuid(id),
            Uuid::new_v4(),
            vec![
                EventPayload::TurnStarted {
                    task_kind: TaskKind::Regular,
                    turn_id: turn,
                    context_snapshot_id: Uuid::new_v4(),
                    model: ModelIdentity {
                        provider_id: "fixture".into(),
                        model_id: "test".into(),
                        revision: None,
                    },
                },
                EventPayload::ItemStarted {
                    turn_id: turn,
                    item_id: item,
                    kind: ItemKind::UserMessage,
                },
                EventPayload::TextDelta {
                    turn_id: turn,
                    item_id: item,
                    text: "user evidence".into(),
                },
                EventPayload::ItemCompleted {
                    turn_id: turn,
                    item_id: item,
                },
                EventPayload::TurnFinished {
                    turn_id: turn,
                    outcome: TurnOutcome::Completed,
                },
            ],
        )
        .await
        .unwrap();
    turn
}
#[test]
fn wire_is_closed_and_validates_before_authorization() {
    let value = wire(Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
    let (decoded, binding) = serde_json::from_value::<RollbackRequest>(value.clone())
        .unwrap()
        .decode()
        .unwrap();
    assert_eq!(binding.kind, BindingKind::Note);
    assert_eq!(binding.resource_id, "note-one");
    assert_eq!(decoded.user_turns, 1);
    assert_eq!(
        serde_json::to_value(RollbackResponse {
            schema_version: 1,
            conversation_id: Uuid::from_u128(1),
            request_id: Uuid::from_u128(3),
            watermark: 6
        })
        .unwrap(),
        serde_json::json!({"schemaVersion":1,"conversationId":Uuid::from_u128(1),"requestId":Uuid::from_u128(3),"watermark":6})
    );
    for (key, replacement) in [
        ("schemaVersion", serde_json::json!(2)),
        ("userTurns", serde_json::json!(0)),
        ("conversationId", serde_json::json!(Uuid::nil())),
        ("requestId", serde_json::json!(Uuid::nil())),
        ("expectedLastTurn", serde_json::json!(Uuid::nil())),
        (
            "expectedWatermark",
            serde_json::json!(9_007_199_254_740_991_u64),
        ),
    ] {
        let mut bad = value.clone();
        bad[key] = replacement;
        assert!(matches!(
            serde_json::from_value::<RollbackRequest>(bad)
                .unwrap()
                .decode(),
            Err(Error::InvalidInput)
        ));
    }
    for key in ["model", "events", "force", "tools"] {
        let mut bad = value.clone();
        bad[key] = serde_json::json!([]);
        assert!(serde_json::from_value::<RollbackRequest>(bad).is_err());
    }
    let mut bad = value;
    bad["binding"]["kind"] = serde_json::json!("shell");
    assert!(serde_json::from_value::<RollbackRequest>(bad).is_err());
}
#[tokio::test]
async fn bound_command_commits_once_and_retries_do_not_remove_later_history() {
    let (db, catalog, service, id, turn) = setup().await;
    let request = Uuid::new_v4();
    let value = wire(id, turn, request);
    let response = execute_legacy_for_tests(
        &catalog,
        &service,
        serde_json::from_value(value.clone()).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.watermark, 6);
    assert_eq!(response.request_id, request);
    assert!(
        service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .turn_order()
            .is_empty()
    );
    let later = user_turn(&service, id).await;
    let retry = execute_legacy_for_tests(
        &catalog,
        &service,
        serde_json::from_value(value.clone()).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(retry.watermark, 6);
    assert_eq!(
        service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .turn_order(),
        &[later]
    );
    db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        execute_legacy_for_tests(&catalog, &service, serde_json::from_value(value).unwrap()).await,
        Err(Error::HostUnavailable)
    ));
    assert_eq!(
        service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .seq(),
        11
    );
}
#[tokio::test]
async fn wrong_revoked_missing_and_archived_hosts_cannot_mutate_history() {
    let (db, catalog, service, id, turn) = setup().await;
    let value = wire(id, turn, Uuid::new_v4());
    let mut wrong = value.clone();
    wrong["binding"]["resourceId"] = serde_json::json!("note-two");
    assert!(matches!(
        execute_legacy_for_tests(&catalog, &service, serde_json::from_value(wrong).unwrap()).await,
        Err(Error::HostUnavailable)
    ));
    let mut stale = value.clone();
    stale["expectedWatermark"] = serde_json::json!(4);
    assert!(matches!(
        execute_legacy_for_tests(&catalog, &service, serde_json::from_value(stale).unwrap()).await,
        Err(Error::Conflict)
    ));
    for query in [
        "update agent_conversations set archived=1 where id=?1",
        "delete from agent_conversation_bindings where conversation_id=?1",
    ] {
        db.lock().unwrap().execute(query, [id.to_string()]).unwrap();
        assert!(matches!(
            execute_legacy_for_tests(
                &catalog,
                &service,
                serde_json::from_value(value.clone()).unwrap()
            )
            .await,
            Err(Error::HostUnavailable)
        ));
        assert_eq!(
            db.lock()
                .unwrap()
                .query_row(
                    "select durable_count from agent_conversations where id=?1",
                    [id.to_string()],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            5
        );
        db.lock()
            .unwrap()
            .execute(
                "update agent_conversations set archived=0 where id=?1",
                [id.to_string()],
            )
            .unwrap();
    }
    db.lock().unwrap().execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','note-one')", [id.to_string()]).unwrap();
    db.lock()
        .unwrap()
        .execute("delete from note_pages where id='note-one'", [])
        .unwrap();
    assert!(matches!(
        execute_legacy_for_tests(&catalog, &service, serde_json::from_value(value).unwrap()).await,
        Err(Error::HostUnavailable)
    ));
    assert_eq!(
        service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .seq(),
        5
    );
}

#[tokio::test]
async fn authority_revoked_after_admission_prevents_atomic_commit() {
    let (db, catalog, service, id, turn) = setup().await;
    let (request, binding) =
        serde_json::from_value::<RollbackRequest>(wire(id, turn, Uuid::new_v4()))
            .unwrap()
            .decode()
            .unwrap();
    catalog
        .authorize_rollback_legacy_for_tests(request.conversation_id, request.request_id, binding)
        .await
        .unwrap();
    db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [id.to_string()],
        )
        .unwrap();
    assert!(service.rollback_conversation(request).await.is_err());
    let mut db = db.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select durable_count from agent_conversations where id=?1",
            [id.to_string()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        5
    );
    assert_eq!(
        db.query_row(
            "select count(*) from agent_input_queues where conversation_id=?1",
            [id.to_string()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let page = crate::agent::repository::read_history::<research_agent_core::protocol::AgentEvent>(
        &mut db,
        research_agent_core::conversation::HistoryQuery::new(ConversationId::from_uuid(id), 0, 10)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page.items.len(), 5);
    assert!(
        matches!(page.items[0].payload(), EventPayload::TurnStarted {turn_id, ..} if *turn_id == turn)
    );
    assert!(
        page.items
            .iter()
            .all(|event| !matches!(event.payload(), EventPayload::ConversationRolledBack { .. }))
    );
}

#[tokio::test]
async fn saved_authority_request_cannot_be_rebound_to_another_host() {
    let (_, catalog, service, id, turn) = setup().await;
    let request = Uuid::new_v4();
    let value = wire(id, turn, request);
    let (decoded, binding) = serde_json::from_value::<RollbackRequest>(value.clone())
        .unwrap()
        .decode()
        .unwrap();
    catalog
        .authorize_rollback_legacy_for_tests(decoded.conversation_id, request, binding)
        .await
        .unwrap();
    catalog
        .bind_legacy_for_tests(
            decoded.conversation_id,
            ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "note-two".into(),
            },
        )
        .await
        .unwrap();
    let mut changed = value;
    changed["binding"]["resourceId"] = serde_json::json!("note-two");
    assert!(matches!(
        execute_legacy_for_tests(&catalog, &service, serde_json::from_value(changed).unwrap())
            .await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        service.open(decoded.conversation_id).await.unwrap().seq(),
        5
    );
}
