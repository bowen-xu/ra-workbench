//! Production canonical port contracts, independent of legacy seed helpers.
use super::*;
use crate::agent::model::{
    responses_decoder::ResponsesDecoder,
    responses_metadata::CapturedResponsesInput,
    responses_native::{self, NativeScope},
};
use research_agent_core::{
    model::{
        CapturedHistoryRef, CapturedTurnRef, CompletedModelItem, ContentPart, ModelMessage,
        ModelOutputItem, ModelRequest, ModelStreamEvent, reconstruct_history,
    },
    protocol::{CanonicalItemSource, ItemKind},
};

#[path = "canonical/fault.rs"]
mod fault;

fn decoded(scope: &ResponsesStateScope, item: Value) -> Box<CompletedModelItem> {
    let mut decoder = ResponsesDecoder::new(
        NativeScope {
            model: scope.model.clone(),
            account_scope: Uuid::from_u128(1),
        },
        65536,
        16,
    )
    .unwrap();
    decoder
        .decode(&json!({"type":"response.created","response":{"id":"r1"}}).to_string())
        .unwrap();
    decoder
        .decode(&json!({"type":"response.output_item.done","item":item}).to_string())
        .unwrap()
        .into_iter()
        .find_map(|event| {
            if let ModelStreamEvent::OutputItemDone(item) = event {
                Some(item)
            } else {
                None
            }
        })
        .unwrap()
}
fn accept(
    db: &mut Connection,
    scope: &ResponsesStateScope,
    item: &CompletedModelItem,
    ordinal: u32,
) -> Uuid {
    let id = Uuid::new_v4();
    let (kind, text, phase) = match &item.normalized {
        Some(ModelOutputItem::ReasoningSummary(summary)) => (
            ItemKind::ReasoningSummary {
                parts: summary.parts.clone(),
            },
            summary.text.clone(),
            None,
        ),
        Some(ModelOutputItem::Message(message)) => (
            ItemKind::AssistantMessage,
            message.text.clone(),
            message.phase,
        ),
        Some(ModelOutputItem::ToolCall(call)) => (
            ItemKind::ToolCall {
                call_id: Uuid::new_v4(),
                name: call.name.clone(),
                provider_call_id: Some(call.call_id.clone()),
                namespace: call.namespace.clone(),
            },
            call.raw_arguments_json.clone(),
            None,
        ),
        None => (
            ItemKind::ProviderNative {
                item_type: item.native.as_ref().unwrap().item_type.clone(),
            },
            String::new(),
            None,
        ),
    };
    append(
        db,
        scope,
        EventPayload::ItemAccepted {
            turn_id: scope.turn_id,
            item_id: id,
            kind,
            text,
            phase,
            source: Some(CanonicalItemSource {
                attempt_id: INITIAL_ATTEMPT,
                ordinal,
                provider_item_id: item.native.as_ref().unwrap().item_id.clone(),
                provider_data_json: Some(item.native.as_ref().unwrap().to_json().unwrap()),
            }),
        },
    );
    id
}
fn request(
    db: &Connection,
    scope: &ResponsesStateScope,
    attempt: Uuid,
    turns: Vec<CapturedTurnRef>,
) -> ModelRequest {
    let history = super::recovery::projection(db, scope);
    // Production freezes closed history before admitting this turn and appends
    // its current user input explicitly; strict history reconstruction never
    // treats an active model request as an already closed historical turn.
    let mut closed = research_agent_core::protocol::ConversationProjection::new(
        ConversationId::from_uuid(scope.conversation_id),
    );
    let encoded: Vec<String> = db
        .prepare("select payload_json from agent_items where conversation_id=?1 order by ordinal")
        .unwrap()
        .query_map([scope.conversation_id.to_string()], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for encoded in encoded {
        let event: research_agent_core::protocol::AgentEvent =
            serde_json::from_str(&encoded).unwrap();
        if matches!(event.payload(),EventPayload::TurnStarted{turn_id,..} if *turn_id==scope.turn_id)
        {
            break;
        }
        closed.apply(&event).unwrap();
    }
    let mut messages = reconstruct_history(&closed).unwrap();
    let current = history.turn(scope.turn_id).unwrap();
    for item_id in &current.item_order {
        let item = current.items.get(item_id).unwrap();
        if item.completed && matches!(item.kind, ItemKind::UserMessage) {
            messages.push(ModelMessage::User {
                content: vec![ContentPart::Text(item.text.clone())],
                source_item_id: Some(*item_id),
            });
        }
    }
    ModelRequest {
        request_id: attempt,
        model: scope.model.clone(),
        messages,
        tools: vec![],
        max_output_tokens: 100,
        parallel_tool_calls: false,
        reasoning_effort: None,
        captured_history: Some(CapturedHistoryRef {
            conversation_id: scope.conversation_id,
            through_seq: history.seq(),
            after_seq: 0,
            turns,
        }),
    }
}
fn capture(request: &ModelRequest) -> (Vec<Value>, CapturedResponsesInput) {
    let input = super::inheritance::encode(
        &ResponsesStateScope {
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            model: request.model.clone(),
        },
        request.messages.clone(),
    );
    let captured = responses_native::capture_request(request, input.len(), false, true).unwrap();
    (input, captured)
}
fn start(db: &mut Connection, scope: &ResponsesStateScope, attempt: Uuid) {
    append(
        db,
        scope,
        EventPayload::ModelRequestStarted {
            turn_id: scope.turn_id,
            attempt_id: attempt,
            model: scope.model.clone(),
        },
    );
}
fn close(db: &mut Connection, scope: &ResponsesStateScope) {
    let started: bool = db.query_row("select exists(select 1 from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='modelResponseStarted' and json_extract(payload_json,'$.payload.turnId')=?2)",params![scope.conversation_id.to_string(),scope.turn_id.to_string()],|row|row.get(0)).unwrap();
    if !started {
        response(db, scope);
    }
    append(
        db,
        scope,
        EventPayload::ModelRequestFinished {
            turn_id: scope.turn_id,
            attempt_id: INITIAL_ATTEMPT,
            input_tokens: None,
            output_tokens: None,
        },
    );
    append(
        db,
        scope,
        EventPayload::TurnFinished {
            turn_id: scope.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
}
fn response(db: &mut Connection, scope: &ResponsesStateScope) {
    append(
        db,
        scope,
        EventPayload::ModelResponseStarted {
            turn_id: scope.turn_id,
            attempt_id: INITIAL_ATTEMPT,
            response_id: "r1".into(),
            model: Some(scope.model.clone()),
        },
    );
}
#[tokio::test]
async fn native_reopen_uses_frozen_canonical_items_without_accumulated_prefix_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("canonical.sqlite");
    let scope = initialize_through(&path, 207);
    let db = connection(&path);
    let user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::UserMessage,
        "read",
    );
    let reasoning = json!({"type":"reasoning","id":"rs","encrypted_content":"opaque-fixture","summary":[],"future":{"v":1}});
    let message = json!({"type":"message","id":"msg","role":"assistant","content":[{"type":"output_text","text":"你🙂\0","annotations":[{"type":"future","v":2}]}]});
    response(&mut db.lock().unwrap(), &scope);
    for (ordinal, item) in [reasoning.clone(), message.clone()].into_iter().enumerate() {
        accept(
            &mut db.lock().unwrap(),
            &scope,
            &decoded(&scope, item),
            ordinal as u32,
        );
    }
    close(&mut db.lock().unwrap(), &scope);
    let next = super::recovery::open_turn(&mut db.lock().unwrap(), &scope, "next");
    let next_user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &next,
        ItemKind::UserMessage,
        "next",
    );
    let attempt = Uuid::new_v4();
    let frozen = request(
        &db.lock().unwrap(),
        &next,
        attempt,
        vec![
            CapturedTurnRef {
                turn_id: scope.turn_id,
                user_item_ids: vec![user],
            },
            CapturedTurnRef {
                turn_id: next.turn_id,
                user_item_ids: vec![next_user],
            },
        ],
    );
    start(&mut db.lock().unwrap(), &next, attempt);
    super::inheritance::item(
        &mut db.lock().unwrap(),
        &next,
        ItemKind::UserMessage,
        "later steer",
    );
    db.lock().unwrap().execute_batch("create trigger reject_prefix before insert on agent_responses_checkpoints begin select raise(abort,'no prefix writes'); end;").unwrap();
    let (input, captured) = capture(&frozen);
    let expected = input.clone();
    drop(db);
    let db = connection(&path);
    let store = SqliteResponsesState::new(db.clone(), next.clone(), 65536).unwrap();
    let prepared = store
        .prepare_canonical_input(Uuid::from_u128(1), next.model.clone(), input, captured)
        .await
        .unwrap();
    assert_eq!(prepared.input, expected);
    assert_eq!(prepared.input[1], reasoning);
    assert_eq!(prepared.input[2], message);
    assert_eq!(prepared.input.len(), 4);
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_responses_checkpoints",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert!(matches!(
        store.preserve(receipt(&next)).await,
        Err(ModelError::UnsupportedCapability)
    ));
}

#[tokio::test]
async fn old_reasoning_only_replays_from_captured_user_anchor_and_preserves_old_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-reasoning.sqlite");
    let scope = initialize_through(&path, 207);
    let db = connection(&path);
    let user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::UserMessage,
        "read",
    );
    let old_request = request(
        &db.lock().unwrap(),
        &scope,
        INITIAL_ATTEMPT,
        vec![CapturedTurnRef {
            turn_id: scope.turn_id,
            user_item_ids: vec![user],
        }],
    );
    let (old_input, _) = capture(&old_request);
    let old = SqliteResponsesState::new(db.clone(), scope.clone(), 65536).unwrap();
    old.fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), old_input.clone())
        .await
        .unwrap();
    let opaque = json!({"type":"reasoning","id":"rs-old","encrypted_content":"old-opaque","summary":[],"unknown":{"retain":true}});
    let mut received = receipt(&scope);
    received.normalized_input = old_input.clone();
    received.input = old_input;
    received.output = vec![opaque.clone()];
    old.fixture_preserve(received).await.unwrap();
    let before: String = db
        .lock()
        .unwrap()
        .query_row(
            "select checkpoint_json from agent_responses_checkpoints where turn_id=?1",
            [scope.turn_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    close(&mut db.lock().unwrap(), &scope);
    let next = super::recovery::open_turn(&mut db.lock().unwrap(), &scope, "next");
    let next_user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &next,
        ItemKind::UserMessage,
        "next",
    );
    let attempt = Uuid::new_v4();
    let frozen = request(
        &db.lock().unwrap(),
        &next,
        attempt,
        vec![
            CapturedTurnRef {
                turn_id: scope.turn_id,
                user_item_ids: vec![user],
            },
            CapturedTurnRef {
                turn_id: next.turn_id,
                user_item_ids: vec![next_user],
            },
        ],
    );
    start(&mut db.lock().unwrap(), &next, attempt);
    let (input, captured) = capture(&frozen);
    let store = SqliteResponsesState::new(db.clone(), next.clone(), 65536).unwrap();
    let prepared = store
        .prepare_canonical_input(
            Uuid::from_u128(1),
            next.model.clone(),
            input.clone(),
            captured.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        prepared.input,
        vec![input[0].clone(), opaque, input[1].clone()]
    );
    assert!(matches!(
        store
            .prepare_canonical_input(Uuid::from_u128(2), next.model.clone(), input, captured)
            .await,
        Err(ModelError::Authentication)
    ));
    let after: String = db
        .lock()
        .unwrap()
        .query_row(
            "select checkpoint_json from agent_responses_checkpoints where turn_id=?1",
            [scope.turn_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_responses_checkpoints",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn old_tool_pair_uses_captured_local_identity_and_preserves_raw_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-tool.sqlite");
    let scope = initialize_through(&path, 207);
    let db = connection(&path);
    let user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::UserMessage,
        "read",
    );
    let first = request(
        &db.lock().unwrap(),
        &scope,
        INITIAL_ATTEMPT,
        vec![CapturedTurnRef {
            turn_id: scope.turn_id,
            user_item_ids: vec![user],
        }],
    );
    let (first_input, _) = capture(&first);
    let old = SqliteResponsesState::new(db.clone(), scope.clone(), 65536).unwrap();
    old.fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), first_input.clone())
        .await
        .unwrap();
    let reasoning =
        json!({"type":"reasoning","id":"rs","encrypted_content":"legacy-opaque","summary":[]});
    let call = json!({"type":"function_call","id":"fc","call_id":"provider-call","name":"read","arguments":"{ }","future":"keep"});
    let mut received = receipt(&scope);
    received.normalized_input = first_input.clone();
    received.input = first_input;
    received.output = vec![reasoning.clone(), call.clone()];
    old.fixture_preserve(received).await.unwrap();
    let pair = Uuid::new_v4();
    let call_item = super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::ToolCall {
            call_id: pair,
            name: "read".into(),
            provider_call_id: Some("provider-call".into()),
            namespace: None,
        },
        "{}",
    );
    super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::ToolResult {
            call_id: pair,
            is_error: Some(false),
        },
        "evidence",
    );
    close(&mut db.lock().unwrap(), &scope);
    let next = super::recovery::open_turn(&mut db.lock().unwrap(), &scope, "next");
    let next_user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &next,
        ItemKind::UserMessage,
        "next",
    );
    let attempt = Uuid::new_v4();
    let frozen = request(
        &db.lock().unwrap(),
        &next,
        attempt,
        vec![
            CapturedTurnRef {
                turn_id: scope.turn_id,
                user_item_ids: vec![user],
            },
            CapturedTurnRef {
                turn_id: next.turn_id,
                user_item_ids: vec![next_user],
            },
        ],
    );
    start(&mut db.lock().unwrap(), &next, attempt);
    let (input, captured) = capture(&frozen);
    assert_eq!(
        input[1]["call_id"],
        research_agent_core::model::history_tool_call_id(call_item)
    );
    let store = SqliteResponsesState::new(db.clone(), next.clone(), 65536).unwrap();
    let prepared = store
        .prepare_canonical_input(
            Uuid::from_u128(1),
            next.model.clone(),
            input.clone(),
            captured,
        )
        .await
        .unwrap();
    assert_eq!(
        prepared.input,
        vec![
            input[0].clone(),
            reasoning,
            call,
            json!({"type":"function_call_output","call_id":"provider-call","output":"evidence"}),
            input[3].clone()
        ]
    );
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_responses_checkpoints",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn old_completed_tool_prefix_accepts_only_proven_bounded_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-complete-output.sqlite");
    let scope = initialize_through(&path, 207);
    let db = connection(&path);
    let user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::UserMessage,
        "read",
    );
    response(&mut db.lock().unwrap(), &scope);
    let pair = Uuid::new_v4();
    let call_item = super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::ToolCall {
            call_id: pair,
            name: "read".into(),
            provider_call_id: Some("provider-call".into()),
            namespace: None,
        },
        "{}",
    );
    let original = "🦀source".repeat(80);
    super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::ToolResult {
            call_id: pair,
            is_error: Some(false),
        },
        &original,
    );
    append(
        &mut db.lock().unwrap(),
        &scope,
        EventPayload::ModelRequestFinished {
            turn_id: scope.turn_id,
            attempt_id: INITIAL_ATTEMPT,
            input_tokens: None,
            output_tokens: None,
        },
    );
    let second_attempt = Uuid::new_v4();
    start(&mut db.lock().unwrap(), &scope, second_attempt);
    append(
        &mut db.lock().unwrap(),
        &scope,
        EventPayload::ModelResponseStarted {
            turn_id: scope.turn_id,
            attempt_id: second_attempt,
            response_id: "second".into(),
            model: Some(scope.model.clone()),
        },
    );
    super::inheritance::item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::AssistantMessage,
        "done",
    );
    append(
        &mut db.lock().unwrap(),
        &scope,
        EventPayload::ModelRequestFinished {
            turn_id: scope.turn_id,
            attempt_id: second_attempt,
            input_tokens: None,
            output_tokens: None,
        },
    );
    append(
        &mut db.lock().unwrap(),
        &scope,
        EventPayload::TurnFinished {
            turn_id: scope.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    let next = super::recovery::open_turn(&mut db.lock().unwrap(), &scope, "next");
    let next_user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &next,
        ItemKind::UserMessage,
        "next",
    );
    let attempt = Uuid::new_v4();
    let mut frozen = request(
        &db.lock().unwrap(),
        &next,
        attempt,
        vec![
            CapturedTurnRef {
                turn_id: scope.turn_id,
                user_item_ids: vec![user],
            },
            CapturedTurnRef {
                turn_id: next.turn_id,
                user_item_ids: vec![next_user],
            },
        ],
    );
    let (full_input, full_captured) = capture(&frozen);
    let mut old_normalized = full_input[..full_input.len() - 1].to_vec();
    let stable_id = research_agent_core::model::history_tool_call_id(call_item);
    for item in &mut old_normalized {
        if item["call_id"] == stable_id {
            item["call_id"] = json!("provider-call");
        }
    }
    let mut old_wire = old_normalized.clone();
    old_wire.insert(
        1,
        json!({"type":"reasoning","id":"legacy","encrypted_content":"opaque","summary":[]}),
    );
    let checkpoint = json!({"schema_version":2,"scope_id":scope.turn_id,"model":scope.model,
        "credential_scope":Uuid::from_u128(1),"checkpoint":{"request_id":second_attempt,
        "actual_model":scope.model,"normalized_prefix":old_normalized,"wire_prefix":old_wire}});
    db.lock().unwrap().execute("insert into agent_responses_checkpoints(turn_id,schema_version,checkpoint_json,updated_at) values (?1,2,?2,'now')", params![scope.turn_id.to_string(), checkpoint.to_string()]).unwrap();
    assert!(research_agent_core::model::tool_output::bound_tool_results(
        &mut frozen.messages,
        64
    ));
    append(
        &mut db.lock().unwrap(),
        &next,
        EventPayload::PhaseChanged {
            turn_id: next.turn_id,
            phase: TurnPhase::RunningModel,
        },
    );
    start(&mut db.lock().unwrap(), &next, attempt);
    let (input, captured) = capture(&frozen);
    let store = SqliteResponsesState::new(db.clone(), next.clone(), 65536).unwrap();
    let baseline = store
        .prepare_canonical_input(
            Uuid::from_u128(1),
            next.model.clone(),
            full_input,
            full_captured,
        )
        .await
        .unwrap();
    assert!(
        baseline
            .input
            .iter()
            .any(|item| item["encrypted_content"] == "opaque")
    );
    assert!(
        baseline
            .input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["output"] == original)
    );
    let mut forged = input.clone();
    forged
        .iter_mut()
        .find(|item| item["type"] == "function_call_output")
        .unwrap()["output"] = json!("forged");
    assert!(
        store
            .prepare_canonical_input(
                Uuid::from_u128(1),
                next.model.clone(),
                forged,
                captured.clone()
            )
            .await
            .is_err()
    );
    let bounded = input
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap()["output"]
        .clone();
    let prepared = store
        .prepare_canonical_input(
            Uuid::from_u128(1),
            next.model.clone(),
            input.clone(),
            captured.clone(),
        )
        .await
        .unwrap();
    assert!(
        prepared
            .input
            .iter()
            .any(|item| item["encrypted_content"] == "opaque")
    );
    assert!(
        prepared
            .input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["output"] == bounded)
    );
    let saved: String = db
        .lock()
        .unwrap()
        .query_row(
            "select checkpoint_json from agent_responses_checkpoints where turn_id=?1",
            [scope.turn_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&saved).unwrap(), checkpoint);
    drop(store);
    drop(db);
    let db = connection(&path);
    let store = SqliteResponsesState::new(db.clone(), next.clone(), 65536).unwrap();
    let replay = store
        .prepare_canonical_input(
            Uuid::from_u128(1),
            next.model.clone(),
            input.clone(),
            captured.clone(),
        )
        .await
        .unwrap();
    assert!(
        replay
            .input
            .iter()
            .any(|item| item["encrypted_content"] == "opaque")
    );
    assert!(
        replay
            .input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["output"] == bounded)
    );
    let token_view = research_agent_core::model::tool_output::truncate_middle_tokens(&original, 12);
    let old_views = [40, 96].map(|budget| {
        research_agent_core::model::tool_output::bounded_tool_output(&original, budget).unwrap()
    });
    for old_view in old_views.into_iter().chain([token_view.clone()]) {
        let mut previous = checkpoint.clone();
        for key in ["normalized_prefix", "wire_prefix"] {
            let item = previous["checkpoint"][key]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|item| item["type"] == "function_call_output")
                .unwrap();
            item["output"] = json!(old_view);
        }
        db.lock()
            .unwrap()
            .execute(
                "update agent_responses_checkpoints set checkpoint_json=?1 where turn_id=?2",
                params![previous.to_string(), scope.turn_id.to_string()],
            )
            .unwrap();
        let result = store
            .prepare_canonical_input(
                Uuid::from_u128(1),
                next.model.clone(),
                input.clone(),
                captured.clone(),
            )
            .await
            .unwrap();
        assert!(
            result
                .input
                .iter()
                .any(|item| item["encrypted_content"] == "opaque")
        );
        assert!(
            result
                .input
                .iter()
                .any(|item| item["type"] == "function_call_output" && item["output"] == bounded)
        );
        let saved: String = db
            .lock()
            .unwrap()
            .query_row(
                "select checkpoint_json from agent_responses_checkpoints where turn_id=?1",
                [scope.turn_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&saved).unwrap(), previous);
    }
    use research_agent_core::conversation::{
        BindingKind, ConversationBinding, ConversationForkRequest,
    };
    {
        let db = db.lock().unwrap();
        crate::notes::register_projection_functions(&db).unwrap();
        crate::storage::schema_compat::register_functions(&db).unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values ('fork-note','Note','now','now')", []).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','fork-note')", [scope.conversation_id.to_string()]).unwrap();
    }
    let child_id = Uuid::new_v4();
    let catalog = crate::agent::repository::SqliteConversationCatalog::new(db.clone());
    catalog
        .fork_legacy_for_tests(ConversationForkRequest {
            child_id: ConversationId::from_uuid(child_id),
            parent_id: ConversationId::from_uuid(scope.conversation_id),
            through_turn_id: scope.turn_id,
            binding: ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "fork-note".into(),
            },
        })
        .await
        .unwrap();
    let child_base = ResponsesStateScope {
        conversation_id: child_id,
        ..scope.clone()
    };
    let child = super::recovery::open_turn(&mut db.lock().unwrap(), &child_base, "child next");
    let child_user = super::inheritance::item(
        &mut db.lock().unwrap(),
        &child,
        ItemKind::UserMessage,
        "child next",
    );
    let child_attempt = Uuid::new_v4();
    let mut child_request = request(
        &db.lock().unwrap(),
        &child,
        child_attempt,
        vec![
            CapturedTurnRef {
                turn_id: scope.turn_id,
                user_item_ids: vec![user],
            },
            CapturedTurnRef {
                turn_id: child.turn_id,
                user_item_ids: vec![child_user],
            },
        ],
    );
    assert!(research_agent_core::model::tool_output::bound_tool_results(
        &mut child_request.messages,
        64
    ));
    append(
        &mut db.lock().unwrap(),
        &child,
        EventPayload::PhaseChanged {
            turn_id: child.turn_id,
            phase: TurnPhase::RunningModel,
        },
    );
    start(&mut db.lock().unwrap(), &child, child_attempt);
    let (child_input, child_capture) = capture(&child_request);
    let child_store = SqliteResponsesState::new(db.clone(), child.clone(), 65536).unwrap();
    let child_result = child_store
        .prepare_canonical_input(
            Uuid::from_u128(1),
            child.model.clone(),
            child_input,
            child_capture,
        )
        .await
        .unwrap();
    let source: String = db.lock().unwrap().query_row(
        "select source_conversation_id from agent_fork_turn_sources where conversation_id=?1 and turn_id=?2",
        params![child_id.to_string(), scope.turn_id.to_string()], |row| row.get(0),
    ).unwrap();
    assert_eq!(source, scope.conversation_id.to_string());
    assert!(
        child_result
            .input
            .iter()
            .any(|item| item["encrypted_content"] == "opaque")
    );
    assert!(
        child_result
            .input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["output"] == bounded)
    );
    let mut corrupt = checkpoint.clone();
    for key in ["normalized_prefix", "wire_prefix"] {
        let item = corrupt["checkpoint"][key]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|item| item["type"] == "function_call_output")
            .unwrap();
        item["output"] = json!(format!("{token_view}x"));
    }
    db.lock()
        .unwrap()
        .execute(
            "update agent_responses_checkpoints set checkpoint_json=?1 where turn_id=?2",
            params![corrupt.to_string(), scope.turn_id.to_string()],
        )
        .unwrap();
    assert!(
        store
            .prepare_canonical_input(Uuid::from_u128(1), next.model, input, captured)
            .await
            .is_err()
    );
}
