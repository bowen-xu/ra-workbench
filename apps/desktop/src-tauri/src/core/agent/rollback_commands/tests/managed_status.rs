use super::managed::{Fixture, fixture, revoke, zero_fixture};
use super::*;
use rusqlite::OpenFlags;

fn request(id: Uuid, operation: Uuid, resource: &str) -> AuthorizationStatusRequest {
    serde_json::from_value(
        serde_json::json!({"schemaVersion":1,"conversationId":id,"requestId":operation,
        "binding":{"kind":"note","resourceId":resource}}),
    )
    .unwrap()
}
fn observer(fixture: &Fixture) -> Connection {
    Connection::open_with_flags(
        fixture._directory.path().join("rollback.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
}
fn heads(observer: &Connection, id: Uuid) -> (u64, u64) {
    observer.query_row("select c.durable_count,s.materialized_position from agent_conversations c cross join data_runtime_state s where c.id=?1 and s.singleton=1", [id.to_string()], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
}
async fn status(fixture: &Fixture, operation: Uuid) -> serde_json::Value {
    serde_json::to_value(
        execute_status(&fixture.catalog, request(fixture.id, operation, "note-one"))
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn zero_fence_status_is_receipt_proof_after_revocation_and_not_live_permission() {
    let fixture = zero_fixture().await;
    let observer = observer(&fixture);
    let operation = Uuid::new_v4();
    let before = heads(&observer, fixture.id);
    let missing = status(&fixture, operation).await;
    assert_eq!(missing["status"]["type"], "noSqlReceipt");
    assert_eq!(missing["status"]["sqlFence"], "0");
    assert_eq!(missing["status"]["closed"], false);
    assert_eq!(heads(&observer, fixture.id), before);
    fixture
        .catalog
        .authorize_rollback(
            ConversationId::from_uuid(fixture.id),
            operation,
            ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "note-one".into(),
            },
        )
        .await
        .unwrap();
    let committed = status(&fixture, operation).await;
    assert_eq!(committed["status"]["type"], "committed");
    assert_eq!(committed["status"]["sqlFence"], "0");
    assert_eq!(committed["status"]["epoch"], missing["status"]["epoch"]);
    revoke(&fixture).await;
    assert_eq!(status(&fixture, operation).await, committed);
    assert!(matches!(
        fixture
            .catalog
            .authorize_rollback(
                ConversationId::from_uuid(fixture.id),
                operation,
                ConversationBinding {
                    kind: BindingKind::Note,
                    resource_id: "note-one".into()
                }
            )
            .await,
        Err(AuthorizationError::Domain(CatalogError::NotFound))
    ));
    assert!(matches!(
        execute_status(&fixture.catalog, request(fixture.id, operation, "note-two")).await,
        Err(Error::Storage(SqlImmediateFailure::Conflict(_)))
    ));
    assert_eq!(heads(&observer, fixture.id), before);
}

#[tokio::test]
async fn status_consumer_does_not_materialize_an_accepted_history_suffix() {
    let fixture = fixture().await;
    let operation = Uuid::new_v4();
    fixture
        .catalog
        .authorize_rollback(
            ConversationId::from_uuid(fixture.id),
            operation,
            ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "note-one".into(),
            },
        )
        .await
        .unwrap();
    let observer = observer(&fixture);
    let id = ConversationId::from_uuid(fixture.id);
    let turn = Uuid::new_v4();
    let item = Uuid::new_v4();
    fixture
        .service
        .commit_payloads(
            id,
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
                EventPayload::PhaseChanged {
                    turn_id: turn,
                    phase: research_agent_core::protocol::TurnPhase::RunningModel,
                },
            ],
        )
        .await
        .unwrap();
    // TurnStarted raises its whole batch to Immediate. Keep the later body
    // mutations in a separate Automatic batch to retain an actual SQL suffix.
    fixture
        .service
        .commit_payloads(
            id,
            Uuid::new_v4(),
            vec![
                EventPayload::ItemStarted {
                    turn_id: turn,
                    item_id: item,
                    kind: ItemKind::AssistantMessage,
                },
                EventPayload::TextDelta {
                    turn_id: turn,
                    item_id: item,
                    text: "accepted pending suffix".into(),
                },
            ],
        )
        .await
        .unwrap();
    let notice = fixture
        .service
        .wait_for_accepted_history(id, 5, 0)
        .await
        .unwrap();
    assert_eq!(notice.watermark, Some(9));
    let before = heads(&observer, fixture.id);
    assert!(
        before.0 < 9,
        "fixture must retain a real unmaterialized suffix"
    );
    let result = status(&fixture, operation).await;
    assert_eq!(result["status"]["type"], "committed");
    assert_eq!(heads(&observer, fixture.id), before);
    assert!(
        fixture
            .service
            .wait_for_accepted_history(id, 9, 0)
            .await
            .unwrap()
            .events
            .is_empty()
    );
}

#[tokio::test]
async fn unmanaged_status_never_invents_an_epoch_or_committed_proof() {
    let (_, catalog, _, id, _) = setup().await;
    let operation = Uuid::new_v4();
    let error = execute_status(&catalog, request(id, operation, "note-one"))
        .await
        .unwrap_err();
    let Error::Storage(SqlImmediateFailure::Unmanaged(meta)) = error else {
        panic!("unmanaged status must be explicit")
    };
    assert_eq!(meta.operation_id, operation);
    assert!(meta.epoch.is_none());
    assert!(meta.sql_fence.is_none());
}
