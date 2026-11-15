use super::*;
use crate::agent::repository::SqliteConversationCatalog;
use research_agent_core::conversation::{
    BindingKind, ConversationBinding, ConversationForkRequest,
};

#[tokio::test]
async fn fork_continuation_sends_inherited_policy_with_child_window_identity() {
    fork_fixture(false).await;
}

#[tokio::test]
async fn fork_continuation_rebuilds_reset_metadata_for_child_model_request() {
    fork_fixture(true).await;
}

async fn fork_fixture(reset: bool) {
    let ((service, store, transport, tools, mut execution, parent), db) =
        setup_with_connection().await;
    buffered(&mut execution, transport.clone(), 120, !reset);
    *transport.responses.lock().unwrap() = VecDeque::from(if reset {
        vec![response(true), response(false), response(false)]
    } else {
        vec![response(false), response(false)]
    });
    {
        let db = db.lock().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values ('note','Note','now','now')", []).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','note')", [parent.to_string()]).unwrap();
    }
    let execution = Arc::new(execution);
    let first = store
        .put(parent, Uuid::new_v4(), input("parent"))
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
    let child = Uuid::new_v4();
    let catalog = SqliteConversationCatalog::new(db.clone());
    catalog
        .fork_legacy_for_tests(ConversationForkRequest {
            parent_id: ConversationId::from_uuid(parent),
            child_id: ConversationId::from_uuid(child),
            through_turn_id: first.turn_id,
            binding: ConversationBinding {
                kind: BindingKind::Note,
                resource_id: "note".into(),
            },
        })
        .await
        .unwrap();
    service
        .open(ConversationId::from_uuid(child))
        .await
        .unwrap();
    let next = store
        .put(child, Uuid::new_v4(), input("child"))
        .await
        .unwrap();
    let child_window = service.active_context_checkpoint(child).await.unwrap();
    assert_eq!(
        run(execution, service.clone(), store, child, next.request_id).await,
        TaskCompletion::Completed
    );
    let sampled_window = service.active_context_checkpoint(child).await.unwrap();
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), if reset { 3 } else { 2 });
    if reset {
        let bodies: Vec<_> = requests
            .last()
            .unwrap()
            .messages
            .iter()
            .filter_map(|message| match message {
                ModelMessage::Developer(body) => {
                    research_agent_core::context::token_budget::TokenBudgetContext::from_body(body)
                        .unwrap()
                }
                _ => None,
            })
            .collect();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].conversation_id, child);
        assert_eq!(bodies[0].window_id, sampled_window.unwrap().window_id);
        // This estimator requires another pre-turn reset in the child. Its
        // predecessor must be the remapped fork checkpoint, not the parent.
        assert_eq!(
            bodies[0].previous_window_id,
            Some(child_window.unwrap().window_id)
        );
        assert!(!requests.last().unwrap().messages.iter().any(|message| matches!(message, ModelMessage::Developer(body) if body.contains(&parent.to_string()))));
    } else {
        assert_eq!(fragments(&requests[1]), vec!["prepare for reset"]);
    }
    assert_eq!(tools.0.load(Ordering::SeqCst), usize::from(reset));
    let db = db.lock().unwrap();
    let roots: Vec<String> = db
        .prepare("select window_id from agent_initial_context_windows order by conversation_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(roots.len(), 2);
    assert_ne!(roots[0], roots[1]);
}
