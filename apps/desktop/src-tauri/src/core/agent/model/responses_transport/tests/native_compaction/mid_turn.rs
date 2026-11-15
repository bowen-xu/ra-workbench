//! Pinned Codex pending_input.rs post-tool auto-compaction contract, adapted
//! to the native owner, real Responses encoder and SQLite (synthetic HTTP only).
use super::*;
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    protocol::ItemKind,
    steering::{SteeringInput, SteeringScope, SteeringStore},
};

struct EvidenceTool(AtomicUsize);
impl ToolExecutor for EvidenceTool {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if call.name == "read" {
            Ok(())
        } else {
            Err(RunError::ToolRejected)
        }
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ToolOutput {
                content: "e".repeat(8000),
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn remote_mid_turn_compaction_continues_tools_before_consuming_steer() {
    exercise(false, false).await;
}

#[tokio::test]
async fn remote_mid_turn_cancel_preserves_evidence_and_pending_steer() {
    exercise(true, false).await;
}

#[tokio::test]
async fn full_window_mid_turn_continues_before_steer_and_restores_canonical_tail() {
    exercise(false, true).await;
}

#[tokio::test]
async fn full_window_mid_turn_cancel_preserves_evidence_and_pending_steer() {
    exercise(true, true).await;
}

async fn exercise(cancel: bool, full_window: bool) {
    exercise_with_resume(cancel, full_window, false, false).await;
}

#[tokio::test]
async fn remote_mid_turn_images_remain_with_user_before_pending_steer() {
    exercise_with_resume(false, false, false, true).await;
}

#[tokio::test]
async fn schema_four_mid_turn_window_resumes_and_recompacts_in_a_fresh_process() {
    exercise_with_resume(false, true, true, false).await;
}

async fn exercise_with_resume(cancel: bool, full_window: bool, restart: bool, with_images: bool) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mid-turn.sqlite3");
    let fixture = if restart {
        let fixture = resume::fixture_at(&path, Uuid::new_v4());
        fixture
            .service
            .create(ConversationId::from_uuid(fixture.id))
            .await
            .unwrap();
        fixture
    } else {
        Fixture::new().await
    };
    let call = json!({"type":"function_call", "id":"tool-item", "call_id":"read1",
        "name":"read", "arguments":"{}"});
    // Normal inference decodes tool calls from completed.output. The V2
    // compaction collector deliberately uses output_item.done instead.
    let tool_response = [
        json!({"type":"response.created","response":{"id":"tool-response","model":"fixture-model"}}),
        json!({"type":"response.output_item.done","item":call}),
        json!({"type":"response.completed","response":{"id":"tool-response",
            "model":"fixture-model","status":"completed","output":[call]}}),
    ].into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
    let provider_window = legacy::full_window("complete supplier window");
    let compact_response = if full_window {
        legacy::json_response(&provider_window)
    } else {
        ok(&events(&[opaque()], true))
    };
    let mut replies = vec![ok(&tool_response), compact_response];
    if !cancel {
        replies.extend([ok(&body()), ok(&body())]);
        if restart {
            replies.extend([
                ok(&body()),
                legacy::json_response(&legacy::full_window("replacement after restart")),
                ok(&body()),
            ]);
        } else {
            replies.push(ok(&body()));
        }
    }
    let (url, peer) = sequential_server(replies).await;
    let record = fixture
        .record_with_context(
            "fresh evidence request",
            vec![research_agent_core::submission::ContextText {
                source_id: "source-1".into(),
                kind: research_agent_core::submission::ContextSourceKind::Paper,
                resource_id: "paper-1".into(),
                revision: "frozen-1".into(),
                anchor: Some("page-1".into()),
                text: "frozen canonical evidence".into(),
            }],
        )
        .await;
    let mut execution = if full_window {
        legacy::legacy_execution(&fixture, &record, &url)
    } else {
        fixture.execution(&record, fixture.transport(&record, &url))
    };
    accounting::account_for_tool_result(&mut execution, fixture.transport(&record, &url));
    execution.max_input_bytes = usize::MAX;
    let tools = Arc::new(EvidenceTool(AtomicUsize::new(0)));
    execution.host_tools = OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read authorized evidence".into(),
            parameters: json!({"type":"object"}),
        }],
        tools.clone(),
    );
    let held = Arc::new(HeldCandidate {
        inner: execution.compaction_strategy.take().unwrap(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    execution.compaction_strategy = Some(held.clone());
    let steering = Arc::new(
        crate::agent::repository::steering::SqliteSteeringStore::new(
            fixture.app.shared_agent_connection(),
        ),
    );
    let images = Arc::new(images::FrozenImages {
        conversation: fixture.id,
        request: record.request_id,
        turn: record.turn_id,
    });
    let execution = Arc::new(execution);
    let admission = if with_images {
        execution
            .start_with_images_and_steering(
                fixture.service.clone(),
                fixture.store.clone(),
                images.clone(),
                ConversationId::from_uuid(fixture.id),
                record.request_id,
                Some(steering.clone()),
            )
            .await
    } else {
        execution
            .start_with_steering(
                fixture.service.clone(),
                fixture.store.clone(),
                ConversationId::from_uuid(fixture.id),
                record.request_id,
                Some(steering.clone()),
            )
            .await
    }
    .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new task")
    };
    timeout(Duration::from_secs(3), held.entered.notified())
        .await
        .unwrap();
    let input_id = Uuid::new_v4();
    steering
        .enqueue(SteeringInput {
            scope: SteeringScope {
                conversation_id: fixture.id,
                turn_id: record.turn_id,
            },
            input_id,
            text: if with_images {
                record.input.text.clone()
            } else {
                "guide after remote compact".into()
            },
            user_input: vec![],
            skill_instructions: vec![],
        })
        .await
        .unwrap();
    if cancel {
        assert!(
            fixture
                .service
                .tasks()
                .cancel_turn(ConversationId::from_uuid(fixture.id), owned.turn_id,)
                .unwrap()
        );
    } else {
        held.release.notify_one();
    }
    assert_eq!(
        timeout(Duration::from_secs(3), owned.task.wait())
            .await
            .unwrap(),
        if cancel {
            TaskCompletion::Canceled
        } else {
            TaskCompletion::Completed
        }
    );
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    if !cancel && !restart {
        let next = fixture.record("next ordinary turn").await;
        let execution = if full_window {
            legacy::legacy_execution(&fixture, &next, &url)
        } else {
            fixture.execution(&next, fixture.transport(&next, &url))
        };
        let completion = if with_images {
            let admission = Arc::new(execution)
                .start_with_images(
                    fixture.service.clone(),
                    fixture.store.clone(),
                    images,
                    ConversationId::from_uuid(fixture.id),
                    next.request_id,
                )
                .await
                .unwrap();
            let SubmissionAdmission::Started(owned) = admission else {
                panic!("new task")
            };
            owned.task.wait().await
        } else {
            fixture
                .start(&next, execution, false)
                .await
                .task
                .wait()
                .await
        };
        let history = fixture
            .service
            .read_since(ConversationId::from_uuid(fixture.id), 0, 500)
            .await
            .unwrap();
        assert_eq!(completion, TaskCompletion::Completed, "{history:?}");
    }
    let window = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap();
    let fixture = if restart {
        let id = fixture.id;
        let original = window.as_ref().unwrap().window_id;
        let connection = Arc::downgrade(&fixture.app.shared_agent_connection());
        drop(owned);
        drop(steering);
        drop(held);
        drop(fixture);
        assert!(
            connection.upgrade().is_none(),
            "parent connection must close"
        );
        resume::run_child(&path, id, &url, original, true).await;
        resume::fixture_at(&path, id)
    } else {
        fixture
    };
    let sent = peer.await.unwrap();
    let source = sent[1]["input"].as_array().unwrap();
    assert!(
        source
            .iter()
            .any(|item| item["type"] == "function_call_output"
                && item["call_id"] == "read1"
                && item["output"].as_str().unwrap().len() == 8000)
    );
    assert!(!sent[1].to_string().contains("guide after remote compact"));
    let history = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    let turn = history.turn(record.turn_id).unwrap();
    assert!(
        turn.items
            .values()
            .any(|item| matches!(item.kind, ItemKind::ToolResult { .. })
                && item.text.len() == 8000
                && item.completed)
    );
    if cancel {
        assert!(window.is_none());
        assert!(!turn.items.contains_key(&input_id));
        let db = fixture.app.connection().unwrap();
        let saved: String = db
            .query_row(
                "select text from agent_steering_inputs where turn_id=?1 and input_id=?2",
                rusqlite::params![record.turn_id.to_string(), input_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, "guide after remote compact");
    } else {
        let window = window.unwrap();
        assert_eq!(window.phase, CompactionPhase::MidTurn);
        assert_eq!(window.schema_version, if full_window { 4 } else { 2 });
        assert!(window.canonical_context.is_some());
        assert!(
            !window
                .canonical_context
                .as_ref()
                .unwrap()
                .messages
                .is_empty()
        );
        assert!(!sent[2].to_string().contains("guide after remote compact"));
        if !with_images {
            assert!(sent[3].to_string().contains("guide after remote compact"));
        }
        for request in &sent[2..if restart { 4 } else { sent.len() }] {
            let input = request["input"].as_array().unwrap();
            if full_window {
                assert!(window.retained.is_empty());
                let canonical = input
                    .iter()
                    .position(|item| item.to_string().contains("frozen canonical evidence"))
                    .unwrap();
                let supplier_user = input
                    .iter()
                    .position(|item| item == &provider_window[0])
                    .unwrap();
                assert!(canonical < supplier_user);
                assert_eq!(
                    &input[supplier_user..supplier_user + provider_window.len()],
                    provider_window.as_slice()
                );
                assert_eq!(
                    input
                        .iter()
                        .filter(|item| item.to_string().contains("frozen canonical evidence"))
                        .count(),
                    1
                );
                assert!(
                    !input
                        .iter()
                        .any(|item| item["type"] == "function_call_output")
                );
                continue;
            }
            assert_eq!(
                input
                    .iter()
                    .filter(|item| item["type"] == "compaction")
                    .count(),
                1
            );
            assert!(input.contains(&opaque()));
            assert!(
                !input
                    .iter()
                    .any(|item| item["type"] == "function_call_output")
            );
            let user = input
                .iter()
                .position(|item| {
                    item["role"] == "user" && item.to_string().contains("fresh evidence request")
                })
                .unwrap();
            let opaque = input
                .iter()
                .position(|item| item["type"] == "compaction")
                .unwrap();
            assert!(user < opaque);
            if with_images {
                let image_users: Vec<_> = input
                    .iter()
                    .filter(|item| {
                        item["content"].as_array().is_some_and(|parts| {
                            parts.iter().any(|part| part["type"] == "input_image")
                        })
                    })
                    .collect();
                assert_eq!(image_users.len(), 1);
                assert_eq!(image_users[0], &input[user]);
                let matching_users = input
                    .iter()
                    .filter(|item| {
                        item["content"].as_array().is_some_and(|parts| {
                            parts.iter().any(|part| {
                                part["type"] == "input_text" && part["text"] == record.input.text
                            })
                        })
                    })
                    .count();
                assert_eq!(matching_users, if request == &sent[2] { 1 } else { 2 });
                assert!(
                    !image_users[0]
                        .to_string()
                        .contains("guide after remote compact")
                );
            }
        }
        assert!(turn.items[&input_id].completed);
        if restart {
            assert_eq!(sent.len(), 7);
            let before_restart = sent[3]["input"].as_array().unwrap();
            for request in &sent[4..6] {
                let input = request["input"].as_array().unwrap();
                // Current base authority may change; the installed history
                // and durable steer must remain the exact ordered prefix.
                assert!(input.len() >= before_restart.len());
                assert_eq!(&input[1..before_restart.len()], &before_restart[1..]);
                let canonical = input
                    .iter()
                    .position(|item| item.to_string().contains("frozen canonical evidence"))
                    .unwrap();
                let user = input
                    .iter()
                    .position(|item| item == &provider_window[0])
                    .unwrap();
                assert!(canonical < user);
                for item in &provider_window {
                    assert_eq!(input.iter().filter(|value| *value == item).count(), 1);
                }
                for text in [
                    "frozen canonical evidence",
                    "guide after remote compact",
                    "resumed fresh",
                ] {
                    assert_eq!(
                        input
                            .iter()
                            .filter(|item| item.to_string().contains(text))
                            .count(),
                        1
                    );
                }
                assert!(
                    !input
                        .iter()
                        .any(|item| item["type"] == "function_call_output")
                );
                assert!(
                    request
                        .to_string()
                        .contains("resumed current research authority")
                );
            }
            let replacement = legacy::full_window("replacement after restart");
            let final_input = sent[6]["input"].as_array().unwrap();
            for item in &replacement {
                assert_eq!(final_input.iter().filter(|value| *value == item).count(), 1);
            }
            for item in &provider_window {
                assert!(!final_input.contains(item));
            }
            assert!(!sent[6].to_string().contains("frozen canonical evidence"));
            assert!(!sent[6].to_string().contains("guide after remote compact"));
            let installed = fixture
                .service
                .active_context_checkpoint(fixture.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(installed.previous_window_id, Some(window.window_id));
            assert_eq!(tools.0.load(Ordering::SeqCst), 1);
            assert!(
                sent.iter()
                    .all(|request| !request.to_string().contains("/compact"))
            );
        }
    }
}
