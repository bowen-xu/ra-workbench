use super::*;
#[path = "mid_turn.rs"]
mod mid_turn;

#[tokio::test]
async fn long_history_compacts_before_the_next_shared_model_request() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    transport
        .responses
        .lock()
        .unwrap()
        .clone_from(&VecDeque::from(vec![
            response(false),
            response(false),
            response(false),
        ]));
    execution.max_input_bytes = usize::MAX;
    execution.transport = token_compaction(transport.clone(), 3000);
    let execution = Arc::new(execution);
    let mut first_input = input("v1");
    first_input.context[0].text = "first evidence ".repeat(1000);
    let first = store.put(id, Uuid::new_v4(), first_input).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id,
        )
        .await,
        TaskCompletion::Completed
    );

    let mut second_input = input("v2");
    second_input.context[0].text = "second evidence ".repeat(500);
    let second = store.put(id, Uuid::new_v4(), second_input).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, second.request_id).await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].tools.is_empty());
    assert!(requests[1].messages.iter().any(|message| matches!(
        message,
        ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("first evidence")))
    )));
    assert!(requests[2].messages.iter().any(|message| matches!(
        message,
        ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("Context checkpoint handoff")))
    )));
}

#[tokio::test]
async fn application_service_installs_the_complete_compaction_window() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let app_store = crate::storage::AppStore::for_test(db, root.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&app_store).unwrap());
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let submissions = Arc::new(SqliteSubmissionStore::new(
        app_store.shared_agent_connection(),
    ));
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from(vec![
            response(false),
            response(false),
            response(false),
            response(false),
        ])),
    });
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: None,
        model: model(),
        compaction_strategy: Some(text_strategy(transport.clone())),
        transport: token_compaction(transport.clone(), 3000),
        host_tools: research_agent_core::tool_registry::OwnedToolRegistration::new(
            vec![ToolDefinition {
                namespace: None,
                name: "read".into(),
                description: "Read authorized source".into(),
                parameters: serde_json::json!({"type":"object"}),
            }],
            Arc::new(Tools(AtomicUsize::new(0))),
        ),
        system_instructions: "Research assistant".into(),
        max_output_tokens: 100,
        max_input_bytes: usize::MAX,
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
    let mut first_input = input("v1");
    first_input.text = "first user".into();
    first_input.context[0].text = "first evidence ".repeat(1000);
    let first = submissions
        .put(id, Uuid::new_v4(), first_input)
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            submissions.clone(),
            id,
            first.request_id,
        )
        .await,
        TaskCompletion::Completed
    );
    let mut second_input = input("v2");
    second_input.text = "second user".into();
    second_input.context[0].text = "second evidence ".repeat(500);
    let second = submissions
        .put(id, Uuid::new_v4(), second_input)
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            submissions.clone(),
            id,
            second.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let mut third_input = input("v3");
    third_input.text = "third user".into();
    let third = submissions
        .put(id, Uuid::new_v4(), third_input)
        .await
        .unwrap();
    assert_eq!(
        run(
            execution,
            service.clone(),
            submissions,
            id,
            third.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    let text = |message: &ModelMessage| match message {
        ModelMessage::User { content: parts, .. } => parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        _ => vec![],
    };
    let compact_input = requests[1]
        .messages
        .iter()
        .flat_map(text)
        .collect::<Vec<_>>();
    assert!(
        compact_input
            .iter()
            .any(|text| text.contains("first evidence"))
    );
    assert!(
        !compact_input
            .iter()
            .any(|text| text.contains("second evidence"))
    );
    let next_input = requests[3]
        .messages
        .iter()
        .flat_map(text)
        .collect::<Vec<_>>();
    for user in ["first user", "second user", "third user"] {
        assert_eq!(
            next_input
                .iter()
                .filter(|text| text.as_str() == user)
                .count(),
            1
        );
    }
    assert!(
        !next_input
            .iter()
            .any(|text| text.contains("first evidence"))
    );
    assert!(
        next_input
            .iter()
            .any(|text| text.contains("second evidence"))
    );
    drop(requests);
    let db = app_store.connection().unwrap();
    let (count, summary_count): (i64, i64) = db
        .query_row(
            "select count(*), count(checkpoint_json) from agent_context_checkpoints",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(summary_count, 1);
    let journal_count: i64 = db
        .query_row("select count(*) from agent_items", [], |row| row.get(0))
        .unwrap();
    assert!(journal_count > 0);
}

#[tokio::test]
async fn failed_compaction_summary_is_audited_without_a_fallback_request() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let app_store = crate::storage::AppStore::for_test(db, root.path().join("objects"));
    let service = Arc::new(AgentSessionService::from_app_store(&app_store).unwrap());
    let id = Uuid::new_v4();
    service.create(ConversationId::from_uuid(id)).await.unwrap();
    let submissions = Arc::new(SqliteSubmissionStore::new(
        app_store.shared_agent_connection(),
    ));
    // Missing completion is a failure; a completed empty suffix is valid.
    let invalid_summary = vec![ModelStreamEvent::Started {
        response_id: "incomplete-summary".into(),
        actual_model: model(),
    }];
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from(vec![
            response(false),
            invalid_summary,
            response(false),
        ])),
    });
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: None,
        model: model(),
        compaction_strategy: Some(text_strategy(transport.clone())),
        transport: token_compaction(transport.clone(), 3000),
        host_tools: research_agent_core::tool_registry::OwnedToolRegistration::new(
            vec![ToolDefinition {
                namespace: None,
                name: "read".into(),
                description: "Read authorized source".into(),
                parameters: serde_json::json!({"type":"object"}),
            }],
            Arc::new(Tools(AtomicUsize::new(0))),
        ),
        system_instructions: "Research assistant".into(),
        max_output_tokens: 100,
        max_input_bytes: usize::MAX,
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
    let mut first_input = input("v1");
    first_input.context[0].text = "first evidence ".repeat(1000);
    let first = submissions
        .put(id, Uuid::new_v4(), first_input)
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            submissions.clone(),
            id,
            first.request_id,
        )
        .await,
        TaskCompletion::Completed
    );
    let mut second_input = input("v2");
    second_input.context[0].text = "second evidence ".repeat(500);
    let second = submissions
        .put(id, Uuid::new_v4(), second_input)
        .await
        .unwrap();
    assert_eq!(
        run(execution, service, submissions, id, second.request_id).await,
        TaskCompletion::Failed
    );

    let db = app_store.connection().unwrap();
    let count: u64 = db
        .query_row(
            "select count(*) from agent_context_checkpoints",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
}

struct HeldStrategy {
    inner: Arc<dyn research_agent_core::context::compaction_strategy::CompactionStrategy>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl research_agent_core::context::compaction_strategy::CompactionStrategy for HeldStrategy {
    fn binding(&self) -> research_agent_core::context::compaction_strategy::CompactionBinding {
        self.inner.binding()
    }
    fn validate_output(
        &self,
        output: &research_agent_core::context::compaction_strategy::CompactionOutput,
    ) -> Result<(), research_agent_core::context::compaction_strategy::CompactionFailure> {
        self.inner.validate_output(output)
    }
    fn compact(
        &self,
        request: research_agent_core::context::compaction_strategy::CompactionRequest,
    ) -> research_agent_core::context::compaction_strategy::CompactionFuture<
        '_,
        research_agent_core::context::compaction_strategy::CompactionCandidate,
    > {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            self.inner.compact(request).await
        })
    }
}

#[tokio::test]
async fn steer_during_pre_turn_compaction_waits_for_fresh_input_sampling() {
    use research_agent_core::steering::{SteeringInput, SteeringScope, SteeringStore};
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
        response(false),
        response(false),
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
    execution.transport = token_compaction(transport.clone(), 3000);
    let execution = Arc::new(execution);
    let mut old = input("old");
    old.context[0].text = "first evidence ".repeat(1000);
    let first = submissions.put(id, Uuid::new_v4(), old).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            submissions.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let mut fresh = input("fresh");
    fresh.context[0].text = "second evidence ".repeat(500);
    fresh.text = "fresh submission".into();
    let second = submissions.put(id, Uuid::new_v4(), fresh).await.unwrap();
    let admission = execution
        .start_with_steering(
            service.clone(),
            submissions,
            ConversationId::from_uuid(id),
            second.request_id,
            Some(steering.clone()),
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), held.entered.notified())
        .await
        .unwrap();
    let steer_id = Uuid::new_v4();
    steering
        .enqueue(SteeringInput {
            scope: SteeringScope {
                conversation_id: id,
                turn_id: second.turn_id,
            },
            input_id: steer_id,
            text: "steer during compact".into(),
            user_input: vec![],
            skill_instructions: vec![],
        })
        .await
        .unwrap();
    held.release.notify_one();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(3), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Completed
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        let has_steer = |request: &ModelRequest| {
            request.messages.iter().any(|message| matches!(message, ModelMessage::User { content: parts, .. } if parts.iter().any(|part| matches!(part, ContentPart::Text(text) if text == "steer during compact"))))
        };
        assert!(!has_steer(&requests[1])); // frozen compaction input
        assert!(!has_steer(&requests[2])); // first sampling owns fresh submission
        assert!(has_steer(&requests[3]));
    }
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(history.turn(second.turn_id).unwrap().items[&steer_id].completed);
}

#[path = "steer_only.rs"]
mod steer_only;
