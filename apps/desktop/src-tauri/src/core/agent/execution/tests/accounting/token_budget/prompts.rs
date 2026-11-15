use super::*;

#[path = "prompts/fork.rs"]
mod fork;

fn next_batch() -> Vec<ModelStreamEvent> {
    let mut events = response(true);
    for event in &mut events {
        match event {
            ModelStreamEvent::ToolCallStarted { call_id, .. }
            | ModelStreamEvent::ToolArgumentsDelta { call_id, .. }
            | ModelStreamEvent::ToolCallCompleted { call_id } => *call_id = "read-2".into(),
            _ => {}
        }
    }
    events
}
fn fragments(request: &ModelRequest) -> Vec<&str> {
    request
        .messages
        .iter()
        .filter_map(|message| match message {
            ModelMessage::Developer(text) if !text.starts_with("<context_window>\n") => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn next_turn_restores_persisted_prompts_with_and_without_reopening_owner() {
    for reopen in [false, true] {
        let ((service, store, transport, tools, mut execution, id), db) =
            setup_with_connection().await;
        buffered(&mut execution, transport.clone(), 120, true);
        *transport.responses.lock().unwrap() =
            VecDeque::from(vec![response(false), response(false)]);
        let execution = Arc::new(execution);
        for index in 0..2 {
            if index == 1 && reopen {
                service
                    .release_idle(ConversationId::from_uuid(id))
                    .await
                    .unwrap();
                service.open(ConversationId::from_uuid(id)).await.unwrap();
            }
            let text = if index == 0 { "first" } else { "second" };
            let mut submission = input(text);
            submission.text = text.into();
            let record = store.put(id, Uuid::new_v4(), submission).await.unwrap();
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
        }
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(fragments(&requests[1]), vec!["prepare for reset"]);
        let position = requests[1]
            .messages
            .iter()
            .position(|message| matches!(message, ModelMessage::Developer(body) if body == "prepare for reset"))
            .unwrap();
        assert!(matches!(
            requests[1].messages[position - 1],
            ModelMessage::Assistant { .. } | ModelMessage::CanonicalOutput(_)
        ));
        assert!(requests[1].messages[position+1..].iter().any(|message| matches!(message, ModelMessage::User { content: parts, .. } if parts == &vec![ContentPart::Text("second".into())])));
        assert_eq!(tools.0.load(Ordering::SeqCst), 0);
        let count: u64 = db
            .lock()
            .unwrap()
            .query_row("select count(*) from agent_context_fragments", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, if reopen { 2 } else { 1 });
    }
}

#[tokio::test]
async fn natural_end_persists_prompts_without_sampling_and_storage_failure_is_not_success() {
    for fail in [false, true] {
        let ((service, store, transport, tools, mut execution, id), db) =
            setup_with_connection().await;
        buffered(&mut execution, transport.clone(), 120, true);
        *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
        if fail {
            db.lock().unwrap().execute_batch("create trigger fail_fragment_insert before insert on agent_context_fragments begin select raise(abort, 'fixture write failure'); end;").unwrap();
        }
        let record = store.put(id, Uuid::new_v4(), input("work")).await.unwrap();
        let completion = run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            record.request_id,
        )
        .await;
        if fail {
            assert_ne!(completion, TaskCompletion::Completed);
        } else {
            assert_eq!(completion, TaskCompletion::Completed);
        }
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
        assert_eq!(tools.0.load(Ordering::SeqCst), 0);
        assert!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .is_none()
        );
        let db = db.lock().unwrap();
        let count: u64 = db
            .query_row("select count(*) from agent_context_fragments", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, u64::from(!fail));
        if !fail {
            let body: String = db
                .query_row(
                    "select fragments_json from agent_context_fragments",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let texts: Vec<String> = serde_json::from_str(&body).unwrap();
            assert_eq!(texts, vec!["prepare for reset"]);
        }
    }
}

#[tokio::test]
async fn continuation_delivers_ordered_developer_prompts_once_after_entire_tool_batch() {
    for after in [80, 120] {
        let (service, store, transport, tools, mut execution, id) = setup().await;
        buffered(&mut execution, transport.clone(), after, true);
        execution.transport = Arc::new(Buffered {
            inner: transport.clone(),
            accounting: Arc::new(ContextAccounting {
                model: model(),
                budget: Some(CompactionBudget {
                    context_window_tokens: 200,
                    effective_percent: 95,
                    configured_trigger_tokens: Some(100),
                    output_reserve_tokens: 10,
                }),
                unknown_window_trigger_tokens: None,
                estimator: Arc::new(Tokens { before: 40, after }),
            }),
            policy: Arc::new(
                TokenBudgetPolicy::resolve(TokenBudgetSettings {
                    reminder_threshold_tokens: Some(20),
                    reminder_message_template: "remaining {n_remaining}".into(),
                    auto_compact_fallback_prompt: Some("save progress".into()),
                    auto_compact_fallback_buffer_tokens: Some(30),
                    ..Default::default()
                })
                .unwrap(),
            ),
        });
        *transport.responses.lock().unwrap() =
            VecDeque::from(vec![response(true), next_batch(), response(false)]);
        let record = store.put(id, Uuid::new_v4(), input("work")).await.unwrap();
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
        assert_eq!(tools.0.load(Ordering::SeqCst), 2);
        assert!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .is_none()
        );
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(fragments(&requests[0]).is_empty());
        let expected = if after == 80 {
            vec!["remaining 20"]
        } else {
            vec!["remaining 0", "save progress"]
        };
        assert_eq!(fragments(&requests[1]), expected);
        assert_eq!(fragments(&requests[2]), expected);
        let first_prompt = requests[1]
            .messages
            .iter()
            .position(|message| matches!(message, ModelMessage::Developer(body) if !body.starts_with("<context_window>\n")))
            .unwrap();
        let result = requests[1]
            .messages
            .iter()
            .rposition(|message| matches!(message, ModelMessage::ToolResult { .. }))
            .unwrap();
        assert!(first_prompt > result);
    }
}
