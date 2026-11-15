//! Pinned Codex compact_resume_fork contract: assert actual branch request history.
use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    BindingKind, ConversationBinding, ConversationForkRequest,
};

#[tokio::test]
async fn native_v2_fork_samples_before_after_and_descendant_windows() {
    fork_samples(false, false).await;
}

#[tokio::test]
async fn native_full_window_fork_samples_before_after_and_descendant_windows() {
    fork_samples(true, false).await;
}

#[tokio::test]
async fn native_v2_tool_fork_restores_history_without_reexecution() {
    fork_samples(false, true).await;
}

#[tokio::test]
async fn native_full_window_tool_fork_restores_history_without_reexecution() {
    fork_samples(true, true).await;
}

pub(super) struct CountedTool(pub(super) AtomicUsize);
impl ToolExecutor for CountedTool {
    fn validate(&self, call: &research_agent_core::model::ToolCall) -> Result<(), RunError> {
        if call.name == "read" {
            Ok(())
        } else {
            Err(RunError::ToolRejected)
        }
    }
    fn authorize_and_execute(
        &self,
        call: research_agent_core::model::ToolCall,
    ) -> RunFuture<'_, ToolOutput> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let content = call.arguments["evidence"]
            .as_str()
            .unwrap_or("historical tool evidence")
            .to_owned();
        Box::pin(async move {
            Ok(ToolOutput {
                content,
                is_error: false,
            })
        })
    }
}

pub(super) fn tool_response() -> String {
    tool_response_for("parent-tool-call", json!({}))
}

pub(super) fn tool_response_for(call_id: &str, arguments: Value) -> String {
    let call = json!({"type":"function_call","id":format!("{call_id}-item"),"call_id":call_id,
        "name":"read","arguments":arguments.to_string()});
    [
        json!({"type":"response.created","response":{"id":"tool-response","model":"fixture-model"}}),
        json!({"type":"response.output_item.done","item":call}),
        json!({"type":"response.completed","response":{"id":"tool-response","model":"fixture-model",
            "status":"completed","output":[call]}}),
    ].into_iter().map(|event| format!("data: {event}\n\n")).collect()
}

async fn fork_samples(legacy: bool, with_tools: bool) {
    let mut fixture = Fixture::new().await;
    let parent = fixture.id;
    let db = fixture.app.shared_agent_connection();
    {
        let db = db.lock().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values ('note','Note','now','now')", []).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','note')", [parent.to_string()]).unwrap();
    }
    let catalog = SqliteConversationCatalog::new(db.clone());
    let window = if legacy {
        legacy::full_window("branch window")
    } else {
        vec![opaque()]
    };
    let compact_response = if legacy {
        legacy::json_response(&window)
    } else {
        ok(&events(&window, true))
    };
    let mut replies = vec![
        ok(&body()),
        compact_response,
        ok(&body()),
        ok(&body()),
        ok(&body()),
        ok(&body()),
        ok(&body()),
    ];
    if with_tools {
        replies.insert(0, ok(&tool_response()));
        replies.insert(3, ok(&tool_response()));
        replies.push(ok(&body()));
    }
    let (url, peer) = sequential_server(replies).await;
    let tools = Arc::new(CountedTool(AtomicUsize::new(0)));
    let execute = |fixture: &Fixture, record: &SubmissionRecord| {
        let mut execution = if legacy {
            legacy::legacy_execution(fixture, record, &url)
        } else {
            fixture.execution(record, fixture.transport(record, &url))
        };
        if with_tools {
            execution.host_tools = OwnedToolRegistration::new(
                vec![research_agent_core::model::ToolDefinition {
                    namespace: None,
                    name: "read".into(),
                    description: "Read authorized evidence".into(),
                    parameters: json!({"type":"object"}),
                }],
                tools.clone(),
            );
        }
        execution
    };
    let first = fixture.record("parent before compact").await;
    assert_eq!(
        fixture
            .start(&first, execute(&fixture, &first), false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let compact = fixture.record("/compact").await;
    assert_eq!(
        fixture
            .start(&compact, execute(&fixture, &compact), true)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let original = fixture
        .service
        .active_context_checkpoint(parent)
        .await
        .unwrap()
        .unwrap();
    let later = fixture.record("excluded parent tail").await;
    assert_eq!(
        fixture
            .start(&later, execute(&fixture, &later), false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let before = Uuid::new_v4();
    let after = Uuid::new_v4();
    let descendant = Uuid::new_v4();
    for (child, source, boundary, label) in [
        (before, parent, first.turn_id, "before branch"),
        (after, parent, compact.turn_id, "after branch"),
        (descendant, after, compact.turn_id, "descendant branch"),
    ] {
        catalog
            .fork_legacy_for_tests(ConversationForkRequest {
                child_id: ConversationId::from_uuid(child),
                parent_id: ConversationId::from_uuid(source),
                through_turn_id: boundary,
                binding: ConversationBinding {
                    kind: BindingKind::Note,
                    resource_id: "note".into(),
                },
            })
            .await
            .unwrap();
        assert_eq!(db.lock().unwrap().query_row(
            "select count(*) from agent_responses_checkpoints p join agent_submissions s on s.turn_id=p.turn_id where s.conversation_id=?1",
            [child.to_string()], |row| row.get::<_, usize>(0)).unwrap(), 0);
        let inherited = fixture
            .service
            .active_context_checkpoint(child)
            .await
            .unwrap();
        if child == before {
            assert!(inherited.is_none());
        } else {
            let inherited = inherited.unwrap();
            assert_ne!(inherited.window_id, original.window_id);
            assert!(inherited.output == original.output);
        }
        fixture.id = child;
        let record = fixture.record(label).await;
        assert_eq!(
            fixture
                .start(&record, execute(&fixture, &record), false)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
    }
    // A subsequent child-owned turn must inherit its own confirmed wire state.
    fixture.id = after;
    let next = fixture.record("after branch continuation").await;
    assert_eq!(
        fixture
            .start(&next, execute(&fixture, &next), false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    if with_tools {
        let tail_child = Uuid::new_v4();
        catalog
            .fork_legacy_for_tests(ConversationForkRequest {
                child_id: ConversationId::from_uuid(tail_child),
                parent_id: ConversationId::from_uuid(parent),
                through_turn_id: later.turn_id,
                binding: ConversationBinding {
                    kind: BindingKind::Note,
                    resource_id: "note".into(),
                },
            })
            .await
            .unwrap();
        fixture.id = tail_child;
        let record = fixture.record("branch retaining tool tail").await;
        assert_eq!(
            fixture
                .start(&record, execute(&fixture, &record), false)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
    }
    let mut sent = peer.await.unwrap();
    assert_eq!(
        tools.0.load(Ordering::SeqCst),
        if with_tools { 2 } else { 0 }
    );
    if with_tools {
        assert_eq!(sent.len(), 10);
        assert!(
            sent[1]["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "function_call_output"
                    && item["call_id"] == "parent-tool-call")
        );
        sent.remove(0);
        sent.remove(2);
        let input = sent[3]["input"].as_array().unwrap();
        let calls = input
            .iter()
            .filter(|item| item["type"] == "function_call")
            .collect::<Vec<_>>();
        let results = input
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        assert_eq!(results.len(), 1);
        assert_eq!(calls[0]["call_id"], results[0]["call_id"]);
        assert_ne!(calls[0]["call_id"], "parent-tool-call");
        assert_eq!(results[0]["output"], "historical tool evidence");
        for index in [4, 5, 6] {
            assert!(!sent[index].to_string().contains("historical tool evidence"));
            assert!(
                !sent[index]["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["type"] == "function_call"
                        || item["type"] == "function_call_output")
            );
        }
        let tail = sent.pop().unwrap();
        assert!(tail.to_string().contains("excluded parent tail"));
        let input = tail["input"].as_array().unwrap();
        let calls = input
            .iter()
            .filter(|item| item["type"] == "function_call")
            .collect::<Vec<_>>();
        let results = input
            .iter()
            .filter(|item| item["type"] == "function_call_output")
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1);
        assert_eq!(results.len(), 1);
        assert_eq!(calls[0]["call_id"], results[0]["call_id"]);
        assert_ne!(calls[0]["call_id"], "parent-tool-call");
        assert_ne!(
            calls[0]["call_id"],
            sent[3]["input"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["type"] == "function_call")
                .unwrap()["call_id"]
        );
        assert_eq!(results[0]["output"], "historical tool evidence");
        for item in &window {
            assert_eq!(input.iter().filter(|value| *value == item).count(), 1);
        }
        if legacy {
            assert_eq!(&input[1..1 + window.len()], window.as_slice());
        }
    }
    assert_eq!(sent.len(), 7);
    assert!(sent[3].to_string().contains("parent before compact"));
    for index in [3, 4, 5, 6] {
        assert!(!sent[index].to_string().contains("excluded parent tail"));
        assert!(!sent[index].to_string().contains("/compact"));
        let input = sent[index]["input"].as_array().unwrap();
        if index == 3 {
            assert!(!input.iter().any(|item| item["type"] == "compaction"));
        } else {
            for item in &window {
                assert_eq!(input.iter().filter(|value| *value == item).count(), 1);
            }
            if legacy {
                assert_eq!(&input[1..1 + window.len()], window.as_slice());
            }
        }
    }
    assert!(!sent[5].to_string().contains("after branch"));
    assert!(sent[6].to_string().contains("after branch"));
    assert_eq!(
        fixture
            .service
            .active_context_checkpoint(parent)
            .await
            .unwrap()
            .unwrap()
            .window_id,
        original.window_id
    );
}
