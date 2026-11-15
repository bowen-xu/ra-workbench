//! Closed producer scope -> real SQLite continuation -> old-model HTTP compact.
//! Synthetic accounts and HTTP only; candidates never install themselves.
use super::*;
use crate::agent::{
    model::responses_transport::{
        ResponsesReceiptSink, ResponsesTransport, ResponsesTransportConfig,
    },
    repository::{
        SqliteSubmissionStore,
        responses_state::{ResponsesStateScope, SqliteResponsesState},
    },
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TaskKind, TurnOutcome, TurnPhase},
    submission::*,
};
use serde_json::{Value, json};
use std::{sync::Mutex, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use uuid::Uuid;

fn append(db: &mut rusqlite::Connection, id: Uuid, payload: EventPayload) {
    let count: u64 = db
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let conversation = ConversationId::from_uuid(id);
    let event = AgentEvent::new(conversation, count + 1, 1, Uuid::new_v4(), payload).unwrap();
    crate::agent::repository::append_batch(db, conversation, Uuid::new_v4(), count, &[event])
        .unwrap();
}

async fn peer(response: String, mime: &'static str) -> (String, tokio::task::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            let (start, size) = loop {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let size: usize = headers.lines().find_map(|l| l.strip_prefix("content-length: ")).unwrap().parse().unwrap();
                    if bytes.len() >= end + 4 + size { break (end + 4, size); }
                }
            };
            let body = serde_json::from_slice(&bytes[start..start + size]).unwrap();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
            body
        }).await.unwrap()
    });
    (url, task)
}

#[tokio::test]
async fn closed_old_model_reads_installed_window_without_promoting_compaction_output() {
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("transition.sqlite");
        let mut db = rusqlite::Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let id = Uuid::new_v4();
        crate::agent::repository::create_conversation(
            &mut db,
            ConversationId::from_uuid(id),
            "Transition",
        )
        .unwrap();
        let db = Arc::new(Mutex::new(db));
        let submissions = SqliteSubmissionStore::new(db.clone());
        let old_model = ModelIdentity {
            provider_id: "openai".into(),
            model_id: "gpt-5.6-terra".into(),
            revision: Some("old".into()),
        };
        let old = submissions
            .put(
                id,
                Uuid::new_v4(),
                SubmissionInput {
                    remote_compaction_v2: None,
                    user_input: vec![],
                    skill_instructions: vec![],
                    instructions: vec![],
                    instruction_configs: vec![],
                    skills: vec![],
                    text: "historical evidence".into(),
                    model: old_model.clone(),
                    reasoning_effort: None,
                    service_tier: None,
                    context: vec![],
                },
            )
            .await
            .unwrap();
        let mut frozen = policy("unused", None);
        frozen.binding = native.binding(old_model.clone());
        frozen.authentication = ContextAuthentication::CodexBackend;
        SqliteContextPolicy::new(db.clone(), &old, frozen.clone())
            .unwrap()
            .prepare()
            .await
            .unwrap();
        let old_item =
            json!({"type":"compaction","encrypted_content":"old-state","vendor_extra":{"v":2}});
        let output = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
            CompactionOutput::RemoteOpaque(old_item.clone())
        } else {
            CompactionOutput::RemoteWindow(vec![old_item.clone()])
        };
        let checkpoint = research_agent_core::context::checkpoint::ContextCheckpoint {
            schema_version: 1,
            window_id: Uuid::new_v4(),
            conversation_id: id,
            turn_id: old.turn_id,
            source_watermark: 3,
            previous_window_id: None,
            binding: frozen.binding.clone(),
            credential_scope_id: Some(Uuid::from_u128(1)),
            phase: CompactionPhase::PreTurn,
            output,
            retained: vec![],
            canonical_context: None,
        };
        {
            let mut connection = db.lock().unwrap();
            append(
                &mut connection,
                id,
                EventPayload::TurnStarted {
                    turn_id: old.turn_id,
                    context_snapshot_id: old.context_snapshot_id,
                    model: old_model.clone(),
                    task_kind: TaskKind::Regular,
                },
            );
            append(
                &mut connection,
                id,
                EventPayload::PhaseChanged {
                    turn_id: old.turn_id,
                    phase: TurnPhase::RunningModel,
                },
            );
            let attempt = Uuid::new_v4();
            append(
                &mut connection,
                id,
                EventPayload::ModelRequestStarted {
                    turn_id: old.turn_id,
                    attempt_id: attempt,
                    model: old_model.clone(),
                },
            );
            crate::agent::repository::context_windows::install(&mut connection, &checkpoint)
                .unwrap();
            append(
                &mut connection,
                id,
                EventPayload::ModelRequestFailed {
                    turn_id: old.turn_id,
                    attempt_id: attempt,
                    error: "synthetic closed producer".into(),
                    retryable: false,
                    category: None,
                },
            );
            append(
                &mut connection,
                id,
                EventPayload::TurnFinished {
                    turn_id: old.turn_id,
                    outcome: TurnOutcome::Canceled,
                },
            );
        }
        let mut next_input = old.input.clone();
        next_input.model.revision = Some("new".into());
        next_input.text = "fresh submission excluded from compact".into();
        let current = submissions
            .put(id, Uuid::new_v4(), next_input)
            .await
            .unwrap();
        append(
            &mut db.lock().unwrap(),
            id,
            EventPayload::TurnStarted {
                turn_id: current.turn_id,
                context_snapshot_id: current.context_snapshot_id,
                model: current.input.model.clone(),
                task_kind: TaskKind::Regular,
            },
        );
        let scope = ResponsesStateScope {
            conversation_id: id,
            turn_id: old.turn_id,
            context_snapshot_id: old.context_snapshot_id,
            model: old_model.clone(),
        };
        let receipts = Arc::new(
            SqliteResponsesState::new(db.clone(), scope.clone(), 100_000)
                .unwrap()
                .with_native_read_policy(native),
        );
        let new_item = json!({"type":"compaction","encrypted_content":"candidate-state"});
        let (response, mime) = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
            let events = [
                json!({"type":"response.created","response":{"id":"compact","model":"gpt-5.6-terra"}}),
                json!({"type":"response.output_item.done","output_index":0,"item":new_item}),
                json!({"type":"response.completed","response":{"id":"compact","model":"gpt-5.6-terra","status":"completed","output":[new_item]}}),
            ];
            (
                events
                    .into_iter()
                    .map(|e| format!("data: {e}\n\n"))
                    .collect(),
                "text/event-stream",
            )
        } else {
            (
                json!({"model":"gpt-5.6-terra","output":[new_item]}).to_string(),
                "application/json",
            )
        };
        let (url, server) = peer(response, mime).await;
        let transport = Arc::new(
            ResponsesTransport::synthetic_platform(
                ResponsesTransportConfig {
                    credential_scope_id: Uuid::from_u128(1),
                    provider_id: "openai".into(),
                    service_tier: None,
                    timeout: Duration::from_secs(3),
                    capabilities: ModelCapabilities {
                        vision: false,
                        tools: true,
                        parallel_tools: false,
                        reasoning_efforts: vec![],
                    },
                },
                &url,
                receipts.clone(),
            )
            .unwrap(),
        );
        let strategy = native
            .responses_strategy(transport.clone(), old_model.clone())
            .unwrap();
        let target = native.binding(current.input.model.clone());
        let handoff =
            PreviousModelStrategy::compose(strategy, transport, target.clone(), None, 64, true)
                .unwrap();
        let attempt = Uuid::new_v4();
        let candidate = prepare_compaction(
            &handoff,
            CompactionRequest {
                operation_id: attempt,
                conversation_id: id,
                turn_id: current.turn_id,
                history_revision: 6,
                phase: CompactionPhase::PreTurn,
                binding: target.clone(),
                max_result_bytes: 8192,
                source: ModelRequest {
                    captured_history: None,
                    request_id: attempt,
                    model: current.input.model.clone(),
                    messages: vec![ModelMessage::User {
                        content: vec![ContentPart::Text("historical evidence".into())],
                        source_item_id: None,
                    }],
                    tools: vec![],
                    max_output_tokens: 128,
                    parallel_tool_calls: false,
                    reasoning_effort: None,
                },
            },
        )
        .await
        .unwrap();
        let sent = server.await.unwrap();
        assert_eq!(sent["model"], "gpt-5.6-terra");
        assert!(sent["input"].as_array().unwrap().contains(&old_item));
        assert!(!sent.to_string().contains("fresh submission"));
        assert_eq!(candidate.binding, target);
        assert_eq!(candidate.credential_scope_id, Some(Uuid::from_u128(1)));
        assert_eq!(handoff.execution_model(), old_model);
        assert!(match candidate.output {
            CompactionOutput::RemoteOpaque(item) => item == new_item,
            CompactionOutput::RemoteWindow(items) => items == vec![new_item],
            _ => false,
        });
        assert_eq!(
            serde_json::to_value(
                crate::agent::repository::context_windows::read_active(&db.lock().unwrap(), id)
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(&checkpoint).unwrap()
        );
        // Reopening the closed producer scope restores the installed old state,
        // never the uninstalled candidate returned by either compact endpoint.
        let reopened = SqliteResponsesState::new(
            Arc::new(Mutex::new(rusqlite::Connection::open(path).unwrap())),
            scope,
            100_000,
        )
        .unwrap()
        .with_native_read_policy(native);
        let wire = reopened
            .prepare_canonical_input(
                Uuid::from_u128(1),
                old_model,
                vec![json!({"role":"user","content":"historical evidence"})],
                crate::agent::model::responses_metadata::CapturedResponsesInput {
                    request_id: Uuid::new_v4(),
                    sampling: false,
                    authority_items: 0,
                    text_compaction: false,
                    history: None,
                    items: vec![],
                    users: vec![],
                },
            )
            .await
            .unwrap()
            .input;
        assert!(wire.contains(&old_item));
        assert!(
            !wire
                .iter()
                .any(|item| item["encrypted_content"] == "candidate-state")
        );
    }
}
