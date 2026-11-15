use super::*;
use research_agent_core::steering::{SteeringInput, SteeringScope, SteeringStore};

// Hold the injected strategy at the actual native post-tool-batch boundary.
// Pending input must not enter the frozen candidate or the first continuation.
#[tokio::test]
async fn steering_during_mid_turn_compaction_waits_for_old_tool_followup() {
    exercise(false, false).await;
}

#[tokio::test]
async fn cancellation_during_mid_turn_compaction_retains_tools_and_pending_input() {
    exercise(true, false).await;
}

#[tokio::test]
async fn failed_mid_turn_compaction_retains_tools_and_pending_input_without_followup() {
    exercise(false, true).await;
}

async fn exercise(cancel: bool, fail: bool) {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let app = crate::storage::AppStore::for_test(db, root.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let submissions = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
    let steering = Arc::new(
        crate::agent::repository::steering::SqliteSteeringStore::new(app.shared_agent_connection()),
    );
    let (_, _, transport, _, mut execution, _) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![
        response(true),
        response(fail), // a summary cannot invoke a tool
        response(false),
        response(false),
    ]);
    let held = Arc::new(HeldStrategy {
        inner: text_strategy(transport.clone()),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    execution.compaction_strategy = Some(held.clone());
    execution.max_input_bytes = usize::MAX;
    execution.transport = token_compaction(transport.clone(), 1000);
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read authorized source".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        Arc::new(LargeTools),
    );
    let submission = submissions
        .put(id, Uuid::new_v4(), input("frozen"))
        .await
        .unwrap();
    let admission = Arc::new(execution)
        .start_with_steering(
            service.clone(),
            submissions,
            ConversationId::from_uuid(id),
            submission.request_id,
            Some(steering.clone()),
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), held.entered.notified())
        .await
        .unwrap();
    let steer_id = Uuid::new_v4();
    let scope = SteeringScope {
        conversation_id: id,
        turn_id: submission.turn_id,
    };
    steering
        .enqueue(SteeringInput {
            scope,
            input_id: steer_id,
            text: "guide after compact".into(),
            user_input: vec![],
            skill_instructions: vec![],
        })
        .await
        .unwrap();
    if cancel {
        assert!(
            service
                .tasks()
                .cancel_turn(ConversationId::from_uuid(id), owned.turn_id)
                .unwrap()
        );
    } else {
        held.release.notify_one();
    }
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(3), owned.task.wait())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        if cancel {
            TaskCompletion::Canceled
        } else if fail {
            TaskCompletion::Failed
        } else {
            TaskCompletion::Completed
        }
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            if cancel {
                1
            } else if fail {
                2
            } else {
                4
            }
        );
        if !cancel && !fail {
            let has_steer = |request: &ModelRequest| {
                request.messages.iter().any(|message| {
                    matches!(
                message, ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(
                    part, ContentPart::Text(text) if text == "guide after compact")))
                })
            };
            assert!(!has_steer(&requests[1]));
            assert!(!has_steer(&requests[2]));
            assert!(has_steer(&requests[3]));
            assert!(requests[2].messages.iter().any(|message| matches!(message,
                ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part,
                    ContentPart::Text(text) if text.contains("evidence frozen"))))));
        }
    }
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(submission.turn_id).unwrap();
    assert!(turn.items.values().any(|item| matches!(item.kind, ItemKind::ToolResult { .. }) && item.text.len() == 8000));
    assert_eq!(turn.items.contains_key(&steer_id), !cancel && !fail);
    assert_eq!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_some(),
        !cancel && !fail
    );
    if cancel || fail {
        let db = app.connection().unwrap();
        let saved: String = db
            .query_row(
                "select text from agent_steering_inputs where turn_id=?1 and input_id=?2",
                rusqlite::params![submission.turn_id.to_string(), steer_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, "guide after compact");
    }
}

struct LargeTools;
impl ToolExecutor for LargeTools {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async {
            Ok(ToolOutput {
                content: "x".repeat(8000),
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn model_token_policy_is_shared_by_live_and_reopened_history() {
    use research_agent_core::model::tool_output::{ToolOutputPolicy, truncate_middle_tokens};
    struct PolicyTransport(Arc<Transport>);
    impl ModelTransport for PolicyTransport {
        fn capabilities(&self) -> ModelCapabilities {
            self.0.capabilities()
        }
        fn tool_output_policy(&self) -> Option<ToolOutputPolicy> {
            Some(ToolOutputPolicy::Tokens(10))
        }
        fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
            self.0.start(request)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let sql_path = root.path().join("state.sqlite3");
    let log_path = root.path().join("data-log");
    let objects = root.path().join("objects");
    let mut sql = Connection::open(&sql_path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = crate::data_interface::Database::open(sql, &log_path).unwrap();
    let app = crate::storage::AppStore::for_test_database(database.clone(), objects.clone());
    let ((service, store, transport, _, mut execution, id), shared) = setup_with_app(&app).await;
    execution.transport = Arc::new(PolicyTransport(transport.clone()));
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        Arc::new(LargeTools),
    );
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let expected = truncate_middle_tokens(&"x".repeat(8000), 12);
    let live = transport.requests.lock().unwrap()[1]
        .messages
        .iter()
        .find_map(|message| match message {
            ModelMessage::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(live, expected);
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(
        history
            .turn(first.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| matches!(item.kind, ItemKind::ToolResult { .. })
                && item.text == "x".repeat(8000))
    );
    let mut replay = research_agent_core::model::reconstruct_history(&history).unwrap();
    let result = replay
        .iter_mut()
        .find_map(|message| match message {
            ModelMessage::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .unwrap();
    *result = research_agent_core::model::tool_output::bounded_tool_output(&"x".repeat(8000), 80)
        .unwrap();
    crate::agent::execution::request::project_history_tool_results(
        &mut replay,
        &history,
        Some(ToolOutputPolicy::Tokens(10)),
        10000,
    )
    .unwrap();
    assert!(replay.iter().any(|message| matches!(message, ModelMessage::ToolResult { content, .. } if content == &expected)));
    let result = replay
        .iter_mut()
        .find_map(|message| match message {
            ModelMessage::ToolResult { content, .. } => Some(content),
            _ => None,
        })
        .unwrap();
    *result = "unproven checkpoint output".into();
    crate::agent::execution::request::project_history_tool_results(
        &mut replay,
        &history,
        Some(ToolOutputPolicy::Tokens(10)),
        10000,
    )
    .unwrap();
    assert!(replay.iter().any(|message| matches!(message, ModelMessage::ToolResult { content, .. } if content == "unproven checkpoint output")));
    drop(history);
    drop(store);
    drop(service);
    drop(shared);
    drop(app);
    drop(database);
    let sql = Connection::open(&sql_path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &log_path).unwrap();
    let app = crate::storage::AppStore::for_test_database(reopened, objects);
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
    let second = store.put(id, Uuid::new_v4(), input("v2")).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, second.request_id).await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    assert!(requests[2].messages.iter().any(|message|
        matches!(message, ModelMessage::ToolResult { content, .. } if content == &expected)));
}

#[tokio::test]
async fn tiny_model_view_budget_keeps_receipted_result_after_native_reopen() {
    struct ReceiptTools {
        calls: AtomicUsize,
        receipts: AtomicUsize,
    }
    impl ToolExecutor for ReceiptTools {
        fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
            Ok(())
        }
        fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(ToolOutput {
                    content: "known 🦀 result".into(),
                    is_error: false,
                })
            })
        }
        fn requires_result_receipt(&self, _: &ToolCall) -> bool {
            true
        }
        fn result_committed(&self, _: ToolCall) -> RunFuture<'_, ()> {
            self.receipts.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }
    let root = tempfile::tempdir().unwrap();
    let sql_path = root.path().join("state.sqlite3");
    let log_path = root.path().join("data-log");
    let objects = root.path().join("objects");
    let mut sql = Connection::open(&sql_path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = crate::data_interface::Database::open(sql, &log_path).unwrap();
    let app = crate::storage::AppStore::for_test_database(database.clone(), objects.clone());
    let ((service, store, transport, _, mut execution, id), shared) = setup_with_app(&app).await;
    let tools = Arc::new(ReceiptTools {
        calls: AtomicUsize::new(0),
        receipts: AtomicUsize::new(0),
    });
    execution.limits.tool_output_bytes = 5;
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        tools.clone(),
    );
    let mut execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Failed
    );
    assert_eq!(tools.calls.load(Ordering::SeqCst), 1);
    assert_eq!(tools.receipts.load(Ordering::SeqCst), 1);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    drop(store);
    drop(service);
    drop(shared);
    drop(app);
    drop(database);

    let sql = Connection::open(&sql_path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &log_path).unwrap();
    let app = crate::storage::AppStore::for_test_database(reopened, objects);
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(!history.has_unknown_tool_outcome());
    assert!(
        history
            .turn(first.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| matches!(item.kind, ItemKind::ToolResult { .. })
                && item.text == "known 🦀 result")
    );
    drop(history);
    Arc::get_mut(&mut execution)
        .unwrap()
        .limits
        .tool_output_bytes = 128;
    let second = store.put(id, Uuid::new_v4(), input("v2")).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, second.request_id).await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages.iter().any(|message|
        matches!(message, ModelMessage::ToolResult { content, .. } if content == "known 🦀 result")));
    assert_eq!(tools.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn oversized_tool_output_keeps_full_native_history_and_reopens_with_same_view() {
    struct ChunkedTools;
    impl ToolExecutor for ChunkedTools {
        fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
            Ok(())
        }
        fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
            Box::pin(async {
                Ok(ToolOutput {
                    content: "🦀".repeat(70_000),
                    is_error: false,
                })
            })
        }
    }
    let root = tempfile::tempdir().unwrap();
    let sql_path = root.path().join("state.sqlite3");
    let log_path = root.path().join("data-log");
    let objects = root.path().join("objects");
    let mut sql = Connection::open(&sql_path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = crate::data_interface::Database::open(sql, &log_path).unwrap();
    let app = crate::storage::AppStore::for_test_database(database.clone(), objects.clone());
    let ((service, store, transport, _, mut execution, id), shared) = setup_with_app(&app).await;
    execution.limits.tool_output_bytes = 128;
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        Arc::new(ChunkedTools),
    );
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let view = {
        let sent = transport.requests.lock().unwrap();
        sent[1]
            .messages
            .iter()
            .find_map(|message| match message {
                ModelMessage::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .unwrap()
    };
    assert!(view.len() <= 128 && view.contains("chars truncated"));
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(
        history
            .turn(first.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| matches!(item.kind, ItemKind::ToolResult { .. })
                && item.text == "🦀".repeat(70_000))
    );
    drop(history);
    drop(store);
    drop(service);
    drop(shared);
    drop(app);
    drop(database);
    let sql = Connection::open(&sql_path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &log_path).unwrap();
    let app = crate::storage::AppStore::for_test_database(reopened, objects);
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(
        history
            .turn(first.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| matches!(item.kind, ItemKind::ToolResult { .. })
                && item.text == "🦀".repeat(70_000))
    );
    drop(history);
    let second = store.put(id, Uuid::new_v4(), input("v2")).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, second.request_id).await,
        TaskCompletion::Completed
    );
    let sent = transport.requests.lock().unwrap();
    assert!(sent[2].messages.iter().any(
        |message| matches!(message, ModelMessage::ToolResult { content, .. } if content == &view)
    ));
}
#[tokio::test]
async fn tool_result_growth_compacts_before_the_next_model_request() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    // Leave room for the full source tool catalog, but not its large result.
    execution.max_input_bytes = usize::MAX;
    execution.transport = token_compaction(transport.clone(), 1000);
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read authorized source".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        Arc::new(LargeTools),
    );
    let record = store.put(id, Uuid::new_v4(), input("v1")).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            record.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].tools.is_empty());
        assert!(requests[1].messages.iter().any(|message| matches!(message,
        ModelMessage::ToolResult { content, .. } if content.len() == 8000)));
        check_budget(&requests[2].messages, &requests[2].tools, 4096).unwrap();
        assert!(
            !requests[2]
                .messages
                .iter()
                .any(|message| matches!(message, ModelMessage::ToolResult { .. }))
        );
    }
    let checkpoint = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.schema_version, 2);
    assert!(checkpoint.canonical_context.is_some());
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(history.turn(record.turn_id).unwrap().items.values().any(|item|
        matches!(item.kind, ItemKind::ToolResult { .. }) && item.text.len() == 8000));
}
