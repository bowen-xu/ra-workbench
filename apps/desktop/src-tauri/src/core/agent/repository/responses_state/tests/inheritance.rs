use super::*;
use crate::agent::model::responses_request::{ResponsesEndpointKind, encode_responses_request};
use research_agent_core::{
    model::*,
    protocol::{ConversationProjection, ItemKind},
};

pub(super) fn encode(scope: &ResponsesStateScope, messages: Vec<ModelMessage>) -> Vec<Value> {
    encode_responses_request(
        &ModelRequest {
            captured_history: None,
            request_id: Uuid::new_v4(),
            model: scope.model.clone(),
            messages,
            tools: vec![],
            max_output_tokens: 100,
            parallel_tool_calls: false,
            reasoning_effort: None,
        },
        ModelCapabilities {
            tools: true,
            vision: false,
            parallel_tools: false,
            reasoning_efforts: vec![],
        },
        ResponsesEndpointKind::Platform,
    )
    .unwrap()["input"]
        .as_array()
        .unwrap()
        .clone()
}
pub(super) fn item(
    db: &mut Connection,
    scope: &ResponsesStateScope,
    kind: ItemKind,
    text: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    append(
        db,
        scope,
        EventPayload::ItemStarted {
            turn_id: scope.turn_id,
            item_id: id,
            kind,
        },
    );
    if !text.is_empty() {
        append(
            db,
            scope,
            EventPayload::TextDelta {
                turn_id: scope.turn_id,
                item_id: id,
                text: text.into(),
            },
        );
    }
    append(
        db,
        scope,
        EventPayload::ItemCompleted {
            turn_id: scope.turn_id,
            item_id: id,
        },
    );
    id
}
fn finish_attempt(db: &mut Connection, scope: &ResponsesStateScope, attempt: Uuid) {
    append(
        db,
        scope,
        EventPayload::ModelResponseStarted {
            turn_id: scope.turn_id,
            attempt_id: attempt,
            response_id: attempt.to_string(),
            model: Some(scope.model.clone()),
        },
    );
    append(
        db,
        scope,
        EventPayload::ModelRequestFinished {
            turn_id: scope.turn_id,
            attempt_id: attempt,
            input_tokens: Some(1),
            output_tokens: Some(1),
        },
    );
}

#[tokio::test]
async fn completed_tool_turn_reopens_with_original_wire_and_stable_history_ids() {
    cross_turn(Predecessor::Completed).await;
}

#[tokio::test]
async fn failed_headers_wait_keeps_earlier_confirmed_state_and_failed_user_input() {
    cross_turn(Predecessor::HeadersCanceled).await;
}

#[tokio::test]
async fn canceled_after_tool_result_allows_a_fresh_request_without_opaque_replay() {
    cross_turn(Predecessor::ToolsCompleted).await;
}

#[tokio::test]
async fn canceled_after_tool_error_allows_a_fresh_request_without_opaque_replay() {
    cross_turn(Predecessor::ToolsFailed).await;
}

enum Predecessor {
    Completed,
    HeadersCanceled,
    ToolsCompleted,
    ToolsFailed,
}

async fn cross_turn(predecessor: Predecessor) {
    let failed_headers_wait = matches!(predecessor, Predecessor::HeadersCanceled);
    let stopped_after_tools = matches!(
        predecessor,
        Predecessor::ToolsCompleted | Predecessor::ToolsFailed
    );
    let tool_failed = matches!(predecessor, Predecessor::ToolsFailed);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite3");
    let scope = initialize(&path);
    let db = connection(&path);
    let store = SqliteResponsesState::new(db.clone(), scope.clone(), 65536).unwrap();
    let mut messages = vec![ModelMessage::User {
        content: vec![ContentPart::Text("read".into())],
        source_item_id: None,
    }];
    let first = encode(&scope, messages.clone());
    item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::UserMessage,
        "read",
    );
    store
        .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), first.clone())
        .await
        .unwrap();
    let tool_output = vec![
        json!({"type":"reasoning","id":"reason1","encrypted_content":"opaque-fixture","summary":[]}),
        json!({"type":"function_call","id":"fc1","call_id":"native-call","name":"read","arguments":"{}"}),
    ];
    let mut received = receipt(&scope);
    received.normalized_input = first.clone();
    received.input = first;
    received.output = tool_output;
    store.fixture_preserve(received).await.unwrap();
    finish_attempt(&mut db.lock().unwrap(), &scope, INITIAL_ATTEMPT);
    item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::AssistantMessage,
        "",
    );
    let call_id = Uuid::new_v4();
    let item_id = item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::ToolCall {
            call_id,
            name: "read".into(),
            provider_call_id: Some("native-call".into()),
            namespace: None,
        },
        "{}",
    );
    item(
        &mut db.lock().unwrap(),
        &scope,
        ItemKind::ToolResult {
            call_id,
            is_error: Some(tool_failed),
        },
        "evidence",
    );
    messages.push(ModelMessage::Assistant {
        phase: None,
        content: vec![],
        tool_calls: vec![ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "native-call".into(),
            name: "read".into(),
            arguments: json!({}),
        }],
    });
    messages.push(ModelMessage::ToolResult {
        call_id: "native-call".into(),
        content: "evidence".into(),
        is_error: tool_failed,
    });
    let second = encode(&scope, messages);
    let second_attempt = Uuid::new_v4();
    append(
        &mut db.lock().unwrap(),
        &scope,
        EventPayload::ModelRequestStarted {
            turn_id: scope.turn_id,
            attempt_id: second_attempt,
            model: scope.model.clone(),
        },
    );
    if !stopped_after_tools {
        let expanded = store
            .fixture_prepare_input(Uuid::from_u128(1), scope.model.clone(), second.clone())
            .await
            .unwrap();
        let mut received = receipt(&scope);
        received.request_id = second_attempt;
        received.normalized_input = second;
        received.input = expanded;
        received.output = output()[1..].to_vec();
        store.fixture_preserve(received).await.unwrap();
        finish_attempt(&mut db.lock().unwrap(), &scope, second_attempt);
        item(
            &mut db.lock().unwrap(),
            &scope,
            ItemKind::AssistantMessage,
            "Done",
        );
    }
    append(
        &mut db.lock().unwrap(),
        &scope,
        EventPayload::TurnFinished {
            turn_id: scope.turn_id,
            outcome: if stopped_after_tools {
                TurnOutcome::Canceled
            } else {
                TurnOutcome::Completed
            },
        },
    );
    let mut history = ConversationProjection::new(ConversationId::from_uuid(scope.conversation_id));
    let encoded_events: Vec<String> = db
        .lock()
        .unwrap()
        .prepare("select payload_json from agent_items order by ordinal")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for event in encoded_events {
        history
            .apply(&serde_json::from_str(&event).unwrap())
            .unwrap();
    }
    if failed_headers_wait {
        let mut failed_messages = reconstruct_history(&history).unwrap();
        failed_messages.push(ModelMessage::User {
            content: vec![ContentPart::Text("failed question".into())],
            source_item_id: None,
        });
        let failed = super::recovery::open_turn(&mut db.lock().unwrap(), &scope, "failed question");
        item(
            &mut db.lock().unwrap(),
            &failed,
            ItemKind::UserMessage,
            "failed question",
        );
        append(
            &mut db.lock().unwrap(),
            &failed,
            EventPayload::PhaseChanged {
                turn_id: failed.turn_id,
                phase: TurnPhase::RunningModel,
            },
        );
        append(
            &mut db.lock().unwrap(),
            &failed,
            EventPayload::ModelRequestStarted {
                turn_id: failed.turn_id,
                attempt_id: Uuid::new_v4(),
                model: failed.model.clone(),
            },
        );
        let failed_state = SqliteResponsesState::new(db.clone(), failed.clone(), 65536).unwrap();
        failed_state
            .fixture_prepare_input(
                Uuid::from_u128(1),
                failed.model.clone(),
                encode(&failed, failed_messages),
            )
            .await
            .unwrap();
        append(
            &mut db.lock().unwrap(),
            &failed,
            EventPayload::TurnFinished {
                turn_id: failed.turn_id,
                outcome: TurnOutcome::Canceled,
            },
        );
        history = super::recovery::projection(&db.lock().unwrap(), &scope);
    }
    let mut replay = reconstruct_history(&history).unwrap();
    replay.push(ModelMessage::User {
        content: vec![ContentPart::Text("next".into())],
        source_item_id: None,
    });
    let next = ResponsesStateScope {
        turn_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
        ..scope.clone()
    };
    let input = SubmissionInput {
        remote_compaction_v2: None,
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        text: "next".into(),
        model: scope.model.clone(),
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
    };
    db.lock().unwrap().execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values (?1,?2,?3,?4,1,?5)",
        params![next.conversation_id.to_string(),Uuid::new_v4().to_string(),next.turn_id.to_string(),next.context_snapshot_id.to_string(),serde_json::to_string(&input).unwrap()]).unwrap();
    append(
        &mut db.lock().unwrap(),
        &next,
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: next.turn_id,
            context_snapshot_id: next.context_snapshot_id,
            model: next.model.clone(),
        },
    );
    let normalized = encode(&next, replay);
    assert_eq!(normalized[1]["call_id"], history_tool_call_id(item_id));
    drop(store);
    drop(db);
    let store = SqliteResponsesState::new(connection(&path), next.clone(), 65536).unwrap();
    if stopped_after_tools {
        // A canceled predecessor may have an opaque provider checkpoint, but
        // it is not a confirmed continuation. A fresh request must use the
        // normalized history and never require the old credential scope.
        let fresh = store
            .fixture_prepare_input(Uuid::from_u128(2), next.model.clone(), normalized.clone())
            .await
            .unwrap();
        assert_eq!(fresh, normalized);
        let reopened = SqliteResponsesState::new(connection(&path), next.clone(), 65536).unwrap();
        assert_eq!(
            reopened
                .fixture_prepare_input(Uuid::from_u128(2), next.model.clone(), normalized.clone())
                .await
                .unwrap(),
            normalized
        );
        return;
    }
    assert!(matches!(
        store
            .fixture_prepare_input(Uuid::from_u128(2), next.model.clone(), normalized.clone())
            .await,
        Err(ModelError::Authentication)
    ));
    let restored = store
        .fixture_prepare_input(Uuid::from_u128(1), next.model.clone(), normalized.clone())
        .await
        .unwrap();
    assert_eq!(restored[1]["encrypted_content"], "opaque-fixture");
    assert_eq!(restored[2]["call_id"], "native-call");
    assert_eq!(restored[3]["call_id"], "native-call");
    if tool_failed {
        let error: Value = serde_json::from_str(restored[3]["output"].as_str().unwrap()).unwrap();
        assert_eq!(error, json!({"is_error":true,"content":"evidence"}));
    }
    assert_eq!(restored.last(), normalized.last());
    if failed_headers_wait {
        assert!(
            restored
                .iter()
                .any(|item| item["content"][0]["text"] == "failed question")
        );
    }
    drop(store);
    let reopened = SqliteResponsesState::new(connection(&path), next.clone(), 65536).unwrap();
    assert_eq!(
        reopened
            .fixture_prepare_input(Uuid::from_u128(1), next.model.clone(), normalized.clone())
            .await
            .unwrap(),
        restored
    );
    let mut received = receipt(&next);
    received.request_id = second_attempt;
    received.normalized_input = normalized.clone();
    received.input = restored.clone();
    received.output = output()[1..].to_vec();
    assert!(matches!(
        reopened.fixture_preserve(received).await,
        Err(ModelError::InvalidRequest)
    ));
    let current_attempt = Uuid::new_v4();
    let db = connection(&path);
    append(
        &mut db.lock().unwrap(),
        &next,
        EventPayload::PhaseChanged {
            turn_id: next.turn_id,
            phase: TurnPhase::RunningModel,
        },
    );
    append(
        &mut db.lock().unwrap(),
        &next,
        EventPayload::ModelRequestStarted {
            turn_id: next.turn_id,
            attempt_id: current_attempt,
            model: next.model.clone(),
        },
    );
    let mut received = receipt(&next);
    received.request_id = current_attempt;
    received.normalized_input = normalized;
    received.input = restored;
    received.output = output()[1..].to_vec();
    reopened.fixture_preserve(received).await.unwrap();
    let encoded: String = db
        .lock()
        .unwrap()
        .query_row(
            "select checkpoint_json from agent_responses_checkpoints where turn_id=?1",
            [next.turn_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let saved: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(saved["schema_version"], 2);
    assert_eq!(
        saved["checkpoint"]["request_id"],
        current_attempt.to_string()
    );
    assert!(saved["checkpoint"]["origin_scope_id"].is_null());
    assert_eq!(db.lock().unwrap().query_row(
        "select count(*) from agent_items where json_extract(payload_json,'$.payload.kind.type')='toolCall'", [],
        |r| r.get::<_, u64>(0)).unwrap(), 1);
}
