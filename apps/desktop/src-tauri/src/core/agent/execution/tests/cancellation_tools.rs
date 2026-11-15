use super::*;
use std::sync::atomic::AtomicBool;
use tokio::sync::Notify;

#[derive(Default)]
struct State {
    started: AtomicUsize,
    stream_waiting: Notify,
    stream_dropped: AtomicBool,
    dropped: Notify,
    release: Notify,
    receipts: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
    requests: AtomicUsize,
}

struct HeldStream {
    state: Arc<State>,
    events: VecDeque<ModelStreamEvent>,
}
impl Drop for HeldStream {
    fn drop(&mut self) {
        self.state.stream_dropped.store(true, Ordering::SeqCst);
        self.state.dropped.notify_one();
    }
}
impl ModelStream for HeldStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move {
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            self.state.stream_waiting.notify_one();
            std::future::pending().await
        })
    }
}
struct HeldTransport(Arc<State>);
impl ModelTransport for HeldTransport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: true,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, _: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            self.0.requests.fetch_add(1, Ordering::SeqCst);
            let mut events = vec![ModelStreamEvent::Started {
                response_id: "held".into(),
                actual_model: model(),
            }];
            for (id, arguments) in [
                ("read-a", "{\"parallel\":true}"),
                ("read-b", "{\"parallel\":true}"),
                ("write", "{}"),
            ] {
                events.extend([
                    ModelStreamEvent::ToolCallStarted {
                        call_id: id.into(),
                        name: "read".into(),
                    },
                    ModelStreamEvent::ToolArgumentsDelta {
                        call_id: id.into(),
                        delta: arguments.into(),
                    },
                    ModelStreamEvent::ToolCallCompleted { call_id: id.into() },
                ]);
            }
            Ok(Box::new(HeldStream {
                state: self.0.clone(),
                events: events.into(),
            }) as Box<dyn ModelStream>)
        })
    }
}
struct HeldTools(Arc<State>);
impl ToolExecutor for HeldTools {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn supports_parallel(&self, call: &ToolCall) -> bool {
        call.arguments["parallel"] == true
    }
    fn requires_result_receipt(&self, _: &ToolCall) -> bool {
        true
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            self.0.calls.lock().unwrap().push(call.call_id.clone());
            self.0.started.fetch_add(1, Ordering::SeqCst);
            self.0.release.notified().await;
            Ok(ToolOutput {
                content: format!("finished {}", call.call_id),
                is_error: false,
            })
        })
    }
    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async move {
            self.0.receipts.lock().unwrap().push(call.call_id);
            Ok(())
        })
    }
}

fn open_store(root: &std::path::Path) -> crate::storage::AppStore {
    let events = crate::runtime_events::RuntimeEvents::new(|_, _| Ok(()));
    crate::storage::AppStore::open_with_paths(
        crate::storage::AppStorePaths {
            data_directory: root.to_owned(),
            agent_configuration_directory: root.join("configuration"),
        },
        crate::storage::AppStorePorts {
            native_note_trace: None,
            writing_context_source: None,
            writing_resource_source: Some(crate::agent::writing_resources::test_sources::source()),
            paper_metadata_source: None,
            retrieval_trace_source: None,
            retrieval_preview_source: None,
            semantic_index_source: None,
            paper_portrait_source: None,
            context_catalog_source: crate::context_catalog_app::source(),
            generation_policy_source: None,
            paper_generation_source: None,
            paper_reading_source: None,
            paper_analysis_source: None,
            candidate_recovery_source: None,
            writing_compile_feedback_source: None,
            #[cfg(target_os = "macos")]
            writing_workspace_compile_source: None,
            #[cfg(unix)]
            writing_workspace_source: None,
            text_editors: Arc::new(
                crate::agent::text_tools::editor_barrier::EditorBarrier::for_events(events.clone()),
            ),
            writing_delivery_tools: Arc::new(
                crate::agent::writing_delivery_tool::DeliveryTools::for_events(events.clone()),
            ),
            events,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn cancel_pending_model_stream_drains_native_tool_owners_without_starting_waiters() {
    let root = tempfile::tempdir().unwrap();
    let app = open_store(root.path());
    assert!(app.shared_agent_connection().logged());
    let ((service, store, _, _, mut execution, id), _) = setup_with_app(&app).await;
    let state = Arc::new(State::default());
    execution.transport = Arc::new(HeldTransport(state.clone()));
    execution.compaction_strategy = None;
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Held fixture".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        Arc::new(HeldTools(state.clone())),
    );
    let submission = store
        .put(id, Uuid::new_v4(), input("frozen"))
        .await
        .unwrap();
    let admission = Arc::new(execution)
        .start(
            service.clone(),
            store,
            ConversationId::from_uuid(id),
            submission.request_id,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("expected started task")
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        state.stream_waiting.notified(),
    )
    .await
    .unwrap();
    assert_eq!(state.started.load(Ordering::SeqCst), 2);
    assert!(
        service
            .tasks()
            .cancel_turn(ConversationId::from_uuid(id), owned.turn_id)
            .unwrap()
    );
    tokio::time::timeout(std::time::Duration::from_secs(3), state.dropped.notified())
        .await
        .unwrap();
    assert!(state.stream_dropped.load(Ordering::SeqCst));
    state.release.notify_waiters();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    assert_eq!(*state.calls.lock().unwrap(), ["read-a", "read-b"]);
    assert_eq!(*state.receipts.lock().unwrap(), ["read-a", "read-b"]);
    assert_eq!(state.requests.load(Ordering::SeqCst), 1);
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(submission.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Canceled));
    assert!(turn.pending_tool_receipts.is_empty());
    for text in ["finished read-a", "finished read-b", "tool_not_started"] {
        assert!(
            turn.items.values().any(|item| item.completed
                && item.text.contains(text)
                && matches!(item.kind, ItemKind::ToolResult { .. })),
            "missing {text}"
        );
    }
    service.shutdown().await.unwrap();
    app.shared_agent_connection().shutdown().unwrap();
    drop(service);
    drop(app);
    let reopened = open_store(root.path());
    assert!(reopened.shared_agent_connection().logged());
    let sessions = Arc::new(AgentSessionService::from_app_store(&reopened).unwrap());
    assert_eq!(
        sessions.open(ConversationId::from_uuid(id)).await.unwrap(),
        history
    );
    let (_, _, transport, tools, mut next_execution, _) = setup().await;
    *transport.responses.lock().unwrap() = vec![response(false)].into();
    next_execution.compaction_strategy = None;
    let next_store = Arc::new(SqliteSubmissionStore::new(
        reopened.shared_agent_connection(),
    ));
    let mut next_input = input("after-reopen");
    next_input.text = "continue after cancellation".into();
    let next = next_store
        .put(id, Uuid::new_v4(), next_input)
        .await
        .unwrap();
    assert_eq!(
        run(
            Arc::new(next_execution),
            sessions.clone(),
            next_store,
            id,
            next.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        for (id, expected, is_error) in [
            ("read-a", "finished read-a", false),
            ("read-b", "finished read-b", false),
            ("write", "tool_not_started", true),
        ] {
            let (item_id, _) = turn.items.iter().find(|(_, item)| matches!(
            &item.kind, ItemKind::ToolCall { provider_call_id: Some(provider_id), .. } if provider_id == id
        )).unwrap();
            let calls: Vec<_> = requests[0]
                .messages
                .iter()
                .filter_map(|message| {
                    let ModelMessage::CanonicalOutput(item) = message else {
                        return None;
                    };
                    if item.canonical_item_id != Some(*item_id) {
                        return None;
                    }
                    match &item.normalized {
                        Some(ModelOutputItem::ToolCall(call)) => Some(&call.call_id),
                        _ => None,
                    }
                })
                .collect();
            assert_eq!(calls.len(), 1, "one recovered call for {id}");
            let results = requests[0]
                .messages
                .iter()
                .filter(|message| {
                    matches!(message,
                        ModelMessage::ToolResult { call_id, content, is_error: actual }
                    if call_id == calls[0] && content.contains(expected) && *actual == is_error
                    )
                })
                .count();
            assert_eq!(results, 1, "one recovered result for {id}");
        }
        assert_eq!(tools.0.load(Ordering::SeqCst), 0);
        assert_eq!(*state.calls.lock().unwrap(), ["read-a", "read-b"]);
    }
    sessions.shutdown().await.unwrap();
    reopened.shared_agent_connection().shutdown().unwrap();
}
