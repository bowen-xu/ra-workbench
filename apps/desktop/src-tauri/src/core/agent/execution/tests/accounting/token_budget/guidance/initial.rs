use super::*;

#[tokio::test]
async fn cancel_initial_hint_read_publishes_neither_snapshot_nor_model_request() {
    let ((service, store, transport, tools, mut execution, id), db) = setup_with_connection().await;
    buffered(&mut execution, transport.clone(), 40, false);
    let calls = install_hint_runtime(&service, "hang").await;
    let record = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
    let owner = tokio::spawn(run(
        Arc::new(execution),
        service.clone(),
        store,
        id,
        record.request_id,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        while !calls.path().join("calls").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        service
            .tasks()
            .cancel_turn(ConversationId::from_uuid(id), record.turn_id)
            .unwrap()
    );
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(4), owner)
            .await
            .unwrap()
            .unwrap(),
        TaskCompletion::Canceled
    );
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    let guard = db.lock().unwrap();
    assert_eq!(
        guard
            .query_row(
                "select count(*) from agent_initial_window_metadata",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn initial_metadata_reads_hint_once_across_turns_reopen_and_fork() {
    use crate::agent::repository::SqliteConversationCatalog;
    use research_agent_core::{
        context::token_budget::TokenBudgetContext,
        conversation::{BindingKind, ConversationBinding, ConversationForkRequest},
    };
    let ((service, store, transport, tools, mut execution, id), db) = setup_with_connection().await;
    buffered(&mut execution, transport.clone(), 40, false);
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false); 3]);
    let calls = install_hint_runtime(&service, "text").await;
    {
        let guard = db.lock().unwrap();
        guard.execute("insert into note_pages(id,title,created_at,updated_at) values ('note','Note','now','now')", []).unwrap();
        guard.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','note')", [id.to_string()]).unwrap();
    }
    let execution = Arc::new(execution);
    let mut first_turn = None;
    for _ in 0..2 {
        let record = store
            .put(id, Uuid::new_v4(), input("continue"))
            .await
            .unwrap();
        first_turn.get_or_insert(record.turn_id);
        assert_eq!(
            run(
                execution.clone(),
                service.clone(),
                store.clone(),
                id,
                record.request_id
            )
            .await,
            TaskCompletion::Completed
        );
        service
            .release_idle(ConversationId::from_uuid(id))
            .await
            .unwrap();
        service.open(ConversationId::from_uuid(id)).await.unwrap();
    }
    let child = Uuid::new_v4();
    let request = ConversationForkRequest {
        parent_id: ConversationId::from_uuid(id),
        child_id: ConversationId::from_uuid(child),
        through_turn_id: first_turn.unwrap(),
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "note".into(),
        },
    };
    let catalog = SqliteConversationCatalog::new(db.clone());
    {
        let guard = db.lock().unwrap();
        guard.execute_batch("create trigger fail_snapshot_fork before insert on agent_initial_window_metadata begin select raise(abort,'fixture snapshot fork failure'); end;").unwrap();
    }
    assert_eq!(
        catalog
            .fork_legacy_for_tests(request.clone())
            .await
            .unwrap_err(),
        research_agent_core::conversation::CatalogError::Unavailable
    );
    {
        let guard = db.lock().unwrap();
        let exists: bool = guard
            .query_row(
                "select exists(select 1 from agent_conversations where id=?1)",
                [child.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists);
        guard
            .execute_batch("drop trigger fail_snapshot_fork")
            .unwrap();
    }
    catalog.fork_legacy_for_tests(request).await.unwrap();
    service
        .open(ConversationId::from_uuid(child))
        .await
        .unwrap();
    let record = store
        .put(child, Uuid::new_v4(), input("branch"))
        .await
        .unwrap();
    assert_eq!(
        run(execution, service.clone(), store, child, record.request_id).await,
        TaskCompletion::Completed
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            metadata(&requests[0].messages),
            metadata(&requests[1].messages)
        );
        let bodies = metadata(&requests[2].messages);
        assert_eq!(bodies.len(), 1);
        let decoded = TokenBudgetContext::from_body(&bodies[0]).unwrap().unwrap();
        assert_eq!(decoded.conversation_id, child);
        assert_eq!(decoded.window_id, decoded.first_window_id);
        assert!(decoded.previous_window_id.is_none());
        assert_eq!(decoded.thread_hint.as_deref(), Some(" saved notes "));
    }
    assert_eq!(
        std::fs::read_to_string(calls.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .active_context_checkpoint(child)
            .await
            .unwrap()
            .is_none()
    );
}
