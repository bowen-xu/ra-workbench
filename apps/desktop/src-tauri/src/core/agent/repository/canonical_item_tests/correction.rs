use super::*;

#[test]
fn interleaving_marker_and_early_tool_result_survive_sqlite_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("interleaving.sqlite3");
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let turn = Uuid::from_u128(2);
    let attempt = Uuid::from_u128(4);
    let call = Uuid::from_u128(5);
    let call_item = Uuid::from_u128(6);
    let result_item = Uuid::from_u128(7);
    let text_item = Uuid::from_u128(8);
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "fixture".into(),
        revision: None,
    };
    let payloads = vec![
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            model: model.clone(),
        },
        EventPayload::PhaseChanged {
            turn_id: turn,
            phase: TurnPhase::RunningModel,
        },
        EventPayload::ModelRequestStarted {
            turn_id: turn,
            attempt_id: attempt,
            model,
        },
        EventPayload::ModelOutputInterleavingStarted {
            turn_id: turn,
            attempt_id: attempt,
        },
        EventPayload::ModelResponseStarted {
            turn_id: turn,
            attempt_id: attempt,
            response_id: "r1".into(),
            model: None,
        },
        EventPayload::ItemAccepted {
            turn_id: turn,
            item_id: call_item,
            kind: ItemKind::ToolCall {
                call_id: call,
                name: "read".into(),
                provider_call_id: Some("call".into()),
                namespace: None,
            },
            text: "{}".into(),
            phase: None,
            source: Some(CanonicalItemSource {
                attempt_id: attempt,
                ordinal: 0,
                provider_item_id: Some("call-item".into()),
                provider_data_json: None,
            }),
        },
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: result_item,
            kind: ItemKind::ToolResult {
                call_id: call,
                is_error: Some(false),
            },
        },
        EventPayload::ItemCompleted {
            turn_id: turn,
            item_id: result_item,
        },
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: text_item,
            kind: ItemKind::AssistantMessage,
        },
        EventPayload::ItemAccepted {
            turn_id: turn,
            item_id: text_item,
            kind: ItemKind::AssistantMessage,
            text: "later".into(),
            phase: Some(AssistantMessagePhase::Commentary),
            source: Some(CanonicalItemSource {
                attempt_id: attempt,
                ordinal: 1,
                provider_item_id: Some("message".into()),
                provider_data_json: None,
            }),
        },
    ];
    let events = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(id, index as u64 + 1, 1, Uuid::from_u128(3), payload).unwrap()
        })
        .collect::<Vec<_>>();
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    create_conversation(&mut db, id, "interleaving").unwrap();
    append_batch(&mut db, id, Uuid::new_v4(), 0, &events).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let history =
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 100).unwrap()).unwrap();
    assert_eq!(history.items, events);
    let mut projection = ConversationProjection::new(id);
    for event in &history.items {
        projection.apply(event).unwrap();
    }
    assert_eq!(
        projection.turn(turn).unwrap().item_order,
        [call_item, text_item, result_item]
    );
}

#[test]
fn corrected_assistant_reopens_with_original_draft_events_and_canonical_search_body() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("correction.sqlite3");
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let item_id = Uuid::new_v4();
    let turn_id = Uuid::from_u128(2);
    let attempt_id = Uuid::from_u128(4);
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "fixture".into(),
        revision: None,
    };
    let payloads = [
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id,
            context_snapshot_id: Uuid::new_v4(),
            model: model.clone(),
        },
        EventPayload::PhaseChanged {
            turn_id,
            phase: TurnPhase::RunningModel,
        },
        EventPayload::ModelRequestStarted {
            turn_id,
            attempt_id,
            model,
        },
        EventPayload::ModelResponseStarted {
            turn_id,
            attempt_id,
            response_id: "r1".into(),
            model: None,
        },
        EventPayload::ItemStarted {
            turn_id,
            item_id,
            kind: ItemKind::AssistantMessage,
        },
        EventPayload::ItemPhaseChanged {
            turn_id,
            item_id,
            phase: AssistantMessagePhase::Commentary,
            native: true,
        },
        EventPayload::TextDelta {
            turn_id,
            item_id,
            text: "草稿🙂\0".into(),
        },
    ];
    let mut events = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(id, index as u64 + 1, 1, Uuid::from_u128(3), payload).unwrap()
        })
        .collect::<Vec<_>>();
    events.push(item(id, events.len() as u64 + 1, item_id, "定稿🧪\0"));
    for payload in [
        EventPayload::ModelRequestFinished {
            turn_id,
            attempt_id,
            input_tokens: None,
            output_tokens: None,
        },
        EventPayload::TurnFinished {
            turn_id,
            outcome: TurnOutcome::Completed,
        },
    ] {
        events.push(
            AgentEvent::new(id, events.len() as u64 + 1, 1, Uuid::from_u128(3), payload).unwrap(),
        );
    }
    let mut expected = ConversationProjection::new(id);
    for event in &events {
        expected.apply(event).unwrap();
    }
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    create_conversation(&mut db, id, "canonical correction").unwrap();
    let batch = Uuid::new_v4();
    assert_eq!(
        append_batch(&mut db, id, batch, 0, &events).unwrap(),
        events.len() as u64
    );
    assert_eq!(
        db.query_row("select body,completed from agent_messages", [], |row| Ok((
            row.get::<_, String>(0)?,
            row.get::<_, bool>(1)?
        )))
        .unwrap(),
        ("定稿🧪\0".into(), true)
    );
    assert_eq!(
        db.query_row("select body from agent_search_messages", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "定稿🧪\0"
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        append_batch(&mut db, id, batch, 0, &events).unwrap(),
        events.len() as u64
    );
    let history =
        read_history::<AgentEvent>(&mut db, HistoryQuery::new(id, 0, 500).unwrap()).unwrap();
    assert_eq!(history.items, events);
    let mut replay = ConversationProjection::new(id);
    for event in &history.items {
        replay.apply(event).unwrap();
    }
    assert_eq!(replay, expected);
    let item = &replay.turn(turn_id).unwrap().items[&item_id];
    assert_eq!(item.text, "定稿🧪\0");
    assert_eq!(item.native_phase, Some(AssistantMessagePhase::FinalAnswer));
    assert_eq!(db.query_row("select json_extract(payload_json,'$.payload.type') from agent_journal_records where ordinal=6",[],|row|row.get::<_,String>(0)).unwrap(),"textDelta");
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}
