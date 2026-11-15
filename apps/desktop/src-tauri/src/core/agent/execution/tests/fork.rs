use super::*;
#[path = "fork/compaction.rs"]
mod compaction;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::{
    conversation::{BindingKind, ConversationBinding, ConversationForkRequest},
    submission::{SubmissionError, SubmissionStore},
};

#[tokio::test]
async fn fork_continuation_uses_original_frozen_inputs_without_inheriting_admission() {
    let mut db = Connection::open_in_memory().unwrap();
    // Exercise today's catalog/runtime against today's schema. The historical
    // 0087 backfill is still run explicitly below on live branch snapshots.
    crate::storage::migrate(&mut db).unwrap();
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at) values ('note','Note','now','now')",
        [],
    )
    .unwrap();
    let db = Arc::new(Mutex::new(db));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let store = Arc::new(SqliteSubmissionStore::new(db.clone()));
    let service = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(db.clone(), "fork".into()).unwrap(),
    )));
    let parent = Uuid::new_v4();
    let child = Uuid::new_v4();
    let grandchild = Uuid::new_v4();
    let binding = ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "note".into(),
    };
    catalog
        .create_bound_legacy_for_tests(ConversationId::from_uuid(parent), binding.clone())
        .await
        .unwrap();
    service
        .open(ConversationId::from_uuid(parent))
        .await
        .unwrap();
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(vec![response(false); 4].into()),
    });
    let tools = Arc::new(Tools(AtomicUsize::new(0)));
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: None,
        model: model(),
        compaction_strategy: None,
        transport: transport.clone(),
        host_tools: research_agent_core::tool_registry::OwnedToolRegistration::new(
            vec![],
            tools.clone(),
        ),
        system_instructions: "Research assistant".into(),
        max_output_tokens: 100,
        max_input_bytes: 100_000,
        limits: RunLimits {
            model_calls: Some(3),
            tool_calls: Some(3),
            output_bytes: Some(10_000),
            tool_output_bytes: 10_000,
            response: StreamLimits {
                text_bytes: 10_000,
                argument_bytes: 10_000,
                tool_calls: 3,
            },
        },
    });
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
    catalog
        .fork_legacy_for_tests(ConversationForkRequest {
            child_id: ConversationId::from_uuid(child),
            parent_id: ConversationId::from_uuid(parent),
            through_turn_id: first.turn_id,
            binding: binding.clone(),
        })
        .await
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
    assert_eq!(
        store.get(child, first.request_id).await.unwrap_err(),
        SubmissionError::NotFound
    );
    assert_eq!(
        store.get_by_turn(child, first.turn_id).await.unwrap_err(),
        SubmissionError::NotFound
    );
    assert_eq!(
        store
            .get_history_input(child, later.turn_id)
            .await
            .unwrap_err(),
        SubmissionError::NotFound
    );
    let inherited = store.get_history_input(child, first.turn_id).await.unwrap();
    assert_eq!(inherited.conversation_id, child);
    assert_eq!(inherited.source_conversation_id, parent);
    assert_eq!(inherited.context_snapshot_id, first.context_snapshot_id);
    assert_eq!(inherited.input, first.input);
    service
        .open(ConversationId::from_uuid(child))
        .await
        .unwrap();
    let next = store
        .put(child, Uuid::new_v4(), input("child-v1"))
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
    let text = transport.requests.lock().unwrap()[2]
        .messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::User { content: parts, .. } => Some(
                parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("evidence parent-v1"));
    assert!(text.contains("evidence child-v1"));
    assert!(!text.contains("parent-later"));
    catalog
        .fork_legacy_for_tests(ConversationForkRequest {
            child_id: ConversationId::from_uuid(grandchild),
            parent_id: ConversationId::from_uuid(child),
            through_turn_id: next.turn_id,
            binding,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .get_history_input(grandchild, first.turn_id)
            .await
            .unwrap()
            .source_conversation_id,
        parent
    );
    assert_eq!(
        store
            .get_history_input(grandchild, next.turn_id)
            .await
            .unwrap()
            .source_conversation_id,
        child
    );
    // Simulate the pre-0087 index state and run the actual backfill against branches with live inputs.
    {
        let guard = db.lock().unwrap();
        guard
            .execute_batch("drop table agent_fork_turn_sources")
            .unwrap();
        guard
            .execute_batch(include_str!(
                "../../../../../migrations/0087_agent_fork_turn_sources.sql"
            ))
            .unwrap();
    }
    assert_eq!(
        store
            .get_history_input(grandchild, first.turn_id)
            .await
            .unwrap()
            .input,
        first.input
    );
    service
        .open(ConversationId::from_uuid(grandchild))
        .await
        .unwrap();
    let final_input = store
        .put(grandchild, Uuid::new_v4(), input("grandchild-v1"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            grandchild,
            final_input.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    // A broken inherited snapshot must fail before another model request, never omit its evidence.
    db.lock().unwrap().execute("update agent_fork_turn_sources set context_snapshot_id=?1 where conversation_id=?2 and turn_id=?3",
        rusqlite::params![Uuid::new_v4().to_string(), grandchild.to_string(), first.turn_id.to_string()]).unwrap();
    assert_eq!(
        store
            .get_history_input(grandchild, first.turn_id)
            .await
            .unwrap_err(),
        SubmissionError::Corrupt
    );
    let bad = store
        .put(grandchild, Uuid::new_v4(), input("must-not-send"))
        .await
        .unwrap();
    assert_eq!(
        run(execution, service, store, grandchild, bad.request_id).await,
        TaskCompletion::Failed
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 4);
}
