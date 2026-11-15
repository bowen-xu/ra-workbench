use super::*;
use crate::agent::repository::{append_batch, create_conversation};
use research_agent_core::protocol::{
    EventPayload as P, ItemKind, ModelIdentity, TurnOutcome, TurnPhase,
};
use serde_json::Value;

#[test]
fn resource_limits_reject_oversized_exports_without_truncation() {
    assert_eq!(
        validate_extent(
            MAX_EXPORT_EVENTS,
            MAX_EXPORT_BYTES,
            MAX_EXPORT_EVENTS as u64
        ),
        Ok(())
    );
    assert_eq!(
        validate_extent(MAX_EXPORT_EVENTS + 1, 0, MAX_EXPORT_EVENTS as u64 + 1),
        Err(SnapshotExportError::TooLarge)
    );
    assert_eq!(
        validate_extent(1, MAX_EXPORT_BYTES + 1, 1),
        Err(SnapshotExportError::TooLarge)
    );
    for (count, bytes, watermark) in [(-1, 0, 0), (0, -1, 0), (1, 0, 2)] {
        assert_eq!(
            validate_extent(count, bytes, watermark),
            Err(SnapshotExportError::Corrupt)
        );
    }
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn conversation(n: u128) -> ConversationId {
    ConversationId::from_uuid(id(n))
}
fn setup() -> (Arc<Mutex<Connection>>, SqliteConversationCatalog) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 105);
    let db = Arc::new(Mutex::new(db));
    (db.clone(), SqliteConversationCatalog::new(db))
}
fn seed(db: &mut Connection, owner: ConversationId) -> u64 {
    create_conversation(db, owner, "研究对话").unwrap();
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "requested".into(),
        revision: None,
    };
    let payloads = vec![
        P::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: id(2),
            context_snapshot_id: id(3),
            model: model.clone(),
        },
        P::ItemStarted {
            turn_id: id(2),
            item_id: id(8),
            kind: ItemKind::UserMessage,
        },
        P::TextDelta {
            turn_id: id(2),
            item_id: id(8),
            text: "问题".into(),
        },
        P::ItemCompleted {
            turn_id: id(2),
            item_id: id(8),
        },
        P::PhaseChanged {
            turn_id: id(2),
            phase: TurnPhase::RunningModel,
        },
        P::ModelRequestStarted {
            turn_id: id(2),
            attempt_id: id(4),
            model: model.clone(),
        },
        P::ModelResponseStarted {
            turn_id: id(2),
            attempt_id: id(4),
            response_id: "private-routing-id".into(),
            model: Some(model),
        },
        P::ItemStarted {
            turn_id: id(2),
            item_id: id(9),
            kind: ItemKind::ToolCall {
                call_id: id(7),
                name: "read_source".into(),
                provider_call_id: Some("provider-call-secret-routing".into()),
                namespace: None,
            },
        },
        P::TextDelta {
            turn_id: id(2),
            item_id: id(9),
            text: "{\"path\":\"main.tex\"}".into(),
        },
        P::ItemCompleted {
            turn_id: id(2),
            item_id: id(9),
        },
        P::ModelRequestFinished {
            turn_id: id(2),
            attempt_id: id(4),
            input_tokens: Some(10),
            output_tokens: Some(2),
        },
        P::PhaseChanged {
            turn_id: id(2),
            phase: TurnPhase::RunningTools,
        },
        P::ItemStarted {
            turn_id: id(2),
            item_id: id(10),
            kind: ItemKind::ToolResult {
                call_id: id(7),
                is_error: Some(false),
            },
        },
        P::TextDelta {
            turn_id: id(2),
            item_id: id(10),
            text: "source text".into(),
        },
        P::ItemCompleted {
            turn_id: id(2),
            item_id: id(10),
        },
        P::PhaseChanged {
            turn_id: id(2),
            phase: TurnPhase::RunningModel,
        },
        P::ItemStarted {
            turn_id: id(2),
            item_id: id(11),
            kind: ItemKind::AssistantMessage,
        },
        P::TextDelta {
            turn_id: id(2),
            item_id: id(11),
            text: "回答".into(),
        },
        P::TextDelta {
            turn_id: id(2),
            item_id: id(11),
            text: " 🧪".into(),
        },
    ];
    let events: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(n, p)| AgentEvent::new(owner, n as u64 + 1, 0, id(6), p).unwrap())
        .collect();
    append_batch(db, owner, id(20), 0, &events).unwrap()
}

#[tokio::test]
async fn normalized_export_keeps_order_partial_state_and_ignores_private_transport_tables() {
    let (db, catalog) = setup();
    let owner = conversation(1);
    let count = seed(&mut db.lock().unwrap(), owner);
    db.lock().unwrap().execute("update agent_conversations set created_at='2026-09-10T00:00:00.000Z', updated_at='2026-09-10T00:00:00.000Z'", []).unwrap();
    // A trap table proves the exporter never needs provider/debug records.
    db.lock().unwrap().execute_batch("drop table agent_responses_checkpoints; create table agent_responses_checkpoints(secret text); insert into agent_responses_checkpoints values('never-export-me');").unwrap();
    let snapshot = catalog.export_snapshot(owner).await.unwrap();
    let value = serde_json::to_value(&snapshot).unwrap();
    let encoded = serde_json::to_string(&snapshot).unwrap();
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/agent/conversation-export.json"
    )))
    .unwrap();
    assert_eq!(
        value, fixture,
        "Rust export must match the portable TypeScript fixture"
    );
    assert_eq!(value["format"], "research-assistant.conversation");
    assert_eq!(value["watermark"], count);
    let turn = &value["turns"][0];
    assert!(turn["outcome"].is_null());
    let items = turn["items"].as_array().unwrap();
    assert_eq!(
        items
            .iter()
            .map(|i| i["kind"]["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["userMessage", "toolCall", "toolResult", "assistantMessage"]
    );
    assert_eq!(items[1]["kind"]["callId"], items[2]["kind"]["callId"]);
    assert_eq!(items[3]["text"], "回答 🧪");
    assert_eq!(items[3]["completed"], false);
    assert_eq!(items[3]["interrupted"], false);
    assert_eq!(turn["attempts"][0]["inputTokens"], 10);
    for private in [
        "never-export-me",
        "private-routing-id",
        "provider-call-secret-routing",
        "responseId",
        "providerCallId",
    ] {
        assert!(!encoded.contains(private));
    }
    let mut db = db.lock().unwrap();
    assert_eq!(
        db.query_row("select durable_count from agent_conversations", [], |r| r
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        count
    );
    // Exporting a live transcript did not synthesize an interrupt or claim a writer.
    let ended = AgentEvent::new(
        owner,
        count + 1,
        0,
        id(6),
        P::TurnFinished {
            turn_id: id(2),
            outcome: TurnOutcome::Interrupted,
        },
    )
    .unwrap();
    append_batch(&mut db, owner, id(21), count, &[ended]).unwrap();
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    let later = snapshot_export(&mut db, owner);
    assert_eq!(later["watermark"], count + 1);
    assert_eq!(later["conversation"]["archived"], true);
    assert_eq!(later["turns"][0]["items"][3]["interrupted"], true);
    assert!(value["turns"][0]["outcome"].is_null());
}
fn snapshot_export(db: &mut Connection, owner: ConversationId) -> Value {
    serde_json::to_value(project_snapshot(read_snapshot(db, owner).unwrap()).unwrap()).unwrap()
}

#[test]
fn projection_uses_frozen_rows_after_later_metadata_and_history_writes() {
    let (db, _) = setup();
    let owner = conversation(1);
    let frozen = {
        let mut db = db.lock().unwrap();
        seed(&mut db, owner);
        read_snapshot(&mut db, owner).unwrap()
    };
    let count = frozen.0.durable_count;
    {
        let mut db = db
            .try_lock()
            .expect("read snapshot released the shared lock");
        let event = AgentEvent::new(
            owner,
            count + 1,
            0,
            id(6),
            P::TextDelta {
                turn_id: id(2),
                item_id: id(11),
                text: " later".into(),
            },
        )
        .unwrap();
        append_batch(&mut db, owner, id(22), count, &[event]).unwrap();
        db.execute("update agent_conversations set title='Renamed'", [])
            .unwrap();
    }
    let before = project_snapshot(frozen).unwrap();
    let after = snapshot_export(&mut db.lock().unwrap(), owner);
    assert_eq!(before.watermark, count);
    assert_eq!(before.conversation.title, "研究对话");
    assert_eq!(before.turns[0].items[3].text, "回答 🧪");
    assert_eq!(after["watermark"], count + 1);
    assert_eq!(after["conversation"]["title"], "Renamed");
    assert_eq!(after["turns"][0]["items"][3]["text"], "回答 🧪 later");
}

#[tokio::test]
async fn corrupt_or_mismatched_history_never_produces_a_partial_export() {
    for damage in [
        "delete from agent_items where ordinal=2",
        "update agent_items set schema_version=2 where ordinal=2",
        "update agent_items set payload_json='{}' where ordinal=2",
        "update agent_items set payload_json=replace(payload_json,'00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000000099') where ordinal=0",
    ] {
        let (db, catalog) = setup();
        seed(&mut db.lock().unwrap(), conversation(1));
        // Simulate an externally corrupted/future database. Normal writes are
        // already prevented by the schema's version CHECK constraint.
        {
            let db = db.lock().unwrap();
            db.execute_batch("pragma ignore_check_constraints=on")
                .unwrap();
            db.execute(damage, []).unwrap();
            db.execute_batch("pragma ignore_check_constraints=off")
                .unwrap();
        }
        assert!(matches!(
            catalog.export_snapshot(conversation(1)).await,
            Err(SnapshotExportError::Corrupt)
        ));
    }
}

#[tokio::test]
async fn empty_missing_and_reopened_conversations_are_distinct() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("history.sqlite");
    {
        let mut db = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 105);
        seed(&mut db, conversation(1));
        create_conversation(&mut db, conversation(50), "Empty").unwrap();
    }
    let catalog =
        SqliteConversationCatalog::new(Arc::new(Mutex::new(Connection::open(path).unwrap())));
    assert_eq!(
        catalog
            .export_snapshot(conversation(1))
            .await
            .unwrap()
            .turns
            .len(),
        1
    );
    let empty = catalog.export_snapshot(conversation(50)).await.unwrap();
    assert_eq!(empty.watermark, 0);
    assert!(empty.turns.is_empty());
    assert!(matches!(
        catalog.export_snapshot(conversation(99)).await,
        Err(SnapshotExportError::NotFound)
    ));
}
