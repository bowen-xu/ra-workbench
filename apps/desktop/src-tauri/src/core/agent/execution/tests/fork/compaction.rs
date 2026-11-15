//! Actual native task continuation across branches before/after a replacement.
use super::*;

#[tokio::test]
async fn native_fork_uses_boundary_window_and_never_reexecutes_historical_tools() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at) values ('note','Note','now','now')",
        [],
    )
    .unwrap();
    let app = crate::storage::AppStore::for_test(db, root.path().join("objects"));
    let catalog = SqliteConversationCatalog::new(app.shared_agent_connection());
    let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
    let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
    let parent = Uuid::new_v4();
    let before = Uuid::new_v4();
    let after = Uuid::new_v4();
    let descendant = Uuid::new_v4();
    let binding = ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "note".into(),
    };
    catalog
        .create_bound_legacy_for_tests(ConversationId::from_uuid(parent), binding.clone())
        .await
        .unwrap();
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(
            vec![
                response(true),
                response(false),
                response(false),
                response(false),
                response(false),
                response(false),
                response(false),
            ]
            .into(),
        ),
    });
    let tools = Arc::new(Tools(AtomicUsize::new(0)));
    let (_, _, _, _, mut execution, _) = setup().await;
    execution.transport = transport.clone();
    execution.compaction_strategy = Some(text_strategy(transport.clone()));
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read authorized evidence".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        tools.clone(),
    );
    let execution = Arc::new(execution);
    let first = store
        .put(parent, Uuid::new_v4(), input("parent-v1"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            parent,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let command = store
        .put(parent, Uuid::new_v4(), input("/compact"))
        .await
        .unwrap();
    let admission = execution
        .clone()
        .start_compaction(
            service.clone(),
            store.clone(),
            ConversationId::from_uuid(parent),
            command.request_id,
            None,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new compact task")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    let parent_window = service
        .active_context_checkpoint(parent)
        .await
        .unwrap()
        .unwrap();
    let later = store
        .put(parent, Uuid::new_v4(), input("parent-later"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            parent,
            later.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    for (child, boundary, label) in [
        (before, first.turn_id, "child-before"),
        (after, command.turn_id, "child-after"),
    ] {
        catalog
            .fork_legacy_for_tests(ConversationForkRequest {
                child_id: ConversationId::from_uuid(child),
                parent_id: ConversationId::from_uuid(parent),
                through_turn_id: boundary,
                binding: binding.clone(),
            })
            .await
            .unwrap();
        let window = service.active_context_checkpoint(child).await.unwrap();
        assert_eq!(window.is_some(), child == after);
        if let Some(window) = window {
            assert_ne!(window.window_id, parent_window.window_id);
            assert!(window.output == parent_window.output);
        }
        let inherited_usage = service.context_usage(child, model()).await.unwrap();
        assert_eq!(
            inherited_usage.status,
            crate::agent::context_usage::WindowUsageStatus::Unknown
        );
        assert!(inherited_usage.current_tokens.is_none());
        assert!(inherited_usage.observation.is_none());
        let next = store
            .put(child, Uuid::new_v4(), input(label))
            .await
            .unwrap();
        assert_eq!(
            run(
                execution.clone(),
                service.clone(),
                store.clone(),
                child,
                next.request_id
            )
            .await,
            TaskCompletion::Completed
        );
        let own_usage = service.context_usage(child, model()).await.unwrap();
        assert_eq!(own_usage.conversation_id, child);
        assert_eq!(own_usage.observation.unwrap().sample.turn_id, next.turn_id);
    }
    catalog
        .fork_legacy_for_tests(ConversationForkRequest {
            child_id: ConversationId::from_uuid(descendant),
            parent_id: ConversationId::from_uuid(after),
            through_turn_id: command.turn_id,
            binding,
        })
        .await
        .unwrap();
    let next = store
        .put(descendant, Uuid::new_v4(), input("descendant"))
        .await
        .unwrap();
    assert_eq!(
        run(execution, service, store, descendant, next.request_id).await,
        TaskCompletion::Completed
    );
    assert_eq!(tools.0.load(Ordering::SeqCst), 1);
    let requests = transport.requests.lock().unwrap();
    let text = |index: usize| {
        requests[index]
            .messages
            .iter()
            .flat_map(|message| match message {
                ModelMessage::User { content: parts, .. } => parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => vec![],
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(text(4).contains("evidence parent-v1"));
    assert!(!text(4).contains("Context checkpoint handoff"));
    for index in [5, 6] {
        assert!(text(index).contains("Context checkpoint handoff"));
        assert!(!text(index).contains("evidence parent-v1"));
        assert!(
            !requests[index]
                .messages
                .iter()
                .any(|message| matches!(message, ModelMessage::ToolResult { .. }))
        );
    }
    for index in [4, 5, 6] {
        assert!(!text(index).contains("parent-later"));
    }
    assert!(!text(6).contains("child-after"));
}
