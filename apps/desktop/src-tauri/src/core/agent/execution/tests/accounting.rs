use super::*;
use research_agent_core::context::{
    accounting::{ContextAccounting, ContextTokenEstimator, Utf8JsonTokenEstimator},
    compaction_strategy::CompactionBudget,
};

#[path = "accounting/frozen_policy.rs"]
mod frozen_policy;
#[path = "accounting/model_transition.rs"]
mod model_transition;
#[path = "accounting/previous_model.rs"]
mod previous_model;
#[path = "accounting/token_budget.rs"]
mod token_budget;
#[path = "accounting/usage_rollback.rs"]
mod usage_rollback;

struct LowTokenEstimate;
impl ContextTokenEstimator for LowTokenEstimate {
    fn estimate(&self, _: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
        Some(21_000)
    }
}

#[tokio::test]
async fn byte_size_alone_does_not_trigger_automatic_compaction() {
    let (_, _, transport, _, mut execution, _) = setup().await;
    execution.max_input_bytes = 131_072;
    execution.transport = Arc::new(Accounted {
        inner: transport,
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: model(),
            estimator: Arc::new(LowTokenEstimate),
            budget: Some(CompactionBudget {
                context_window_tokens: 272_000,
                effective_percent: 95,
                configured_trigger_tokens: None,
                output_reserve_tokens: 4096,
            }),
        }),
    });
    let messages = vec![ModelMessage::System("x".repeat(150_000))];
    assert!(check_budget(&messages, &[], execution.max_input_bytes).is_err());
    assert!(
        !execution
            .should_compact_context(&messages, &[], None)
            .unwrap()
    );
}

#[test]
fn production_context_budget_does_not_reintroduce_a_fixed_input_ceiling() {
    let identity = ModelIdentity {
        provider_id: "openai".into(),
        model_id: "gpt-5.6".into(),
        revision: None,
    };
    let profile = crate::agent::execution_profile::execution_profile(
        &identity,
        crate::agent::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .unwrap();
    assert_eq!(profile.max_input_bytes, usize::MAX);
    assert_eq!(profile.limits.tool_output_bytes, 131_072);
}

#[tokio::test]
async fn large_fresh_input_below_token_budget_does_not_compact_or_fail() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false), response(false)]);
    execution.max_input_bytes = usize::MAX;
    execution.transport = Arc::new(Accounted {
        inner: transport.clone(),
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: model(),
            estimator: Arc::new(LowTokenEstimate),
            budget: Some(CompactionBudget {
                context_window_tokens: 272_000,
                effective_percent: 95,
                configured_trigger_tokens: None,
                output_reserve_tokens: 4096,
            }),
        }),
    });
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
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
    let mut fresh = input("second");
    fresh.text = "x".repeat(150_000);
    let second = store.put(id, Uuid::new_v4(), fresh).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, second.request_id).await,
        TaskCompletion::Completed
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

struct PersistenceFailure;
impl ContextTokenEstimator for PersistenceFailure {
    fn estimate(&self, _: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
        Some(100)
    }

    fn prepare_auto_compaction<'a>(
        &'a self,
        _: Option<u64>,
        _: Option<&'a research_agent_core::context::checkpoint::ContextCheckpoint>,
    ) -> ModelFuture<'a, ()> {
        Box::pin(async { Err(ModelError::Persistence) })
    }
}

#[tokio::test]
async fn preparation_persistence_failure_does_not_record_a_model_attempt_or_failed_terminal() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    execution.transport = Arc::new(Accounted {
        inner: transport.clone(),
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: model(),
            budget: None,
            estimator: Arc::new(PersistenceFailure),
        }),
    });
    let submission = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
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
    assert_eq!(owned.task.turn_id(), Some(submission.turn_id));
    let observed = owned.task.observe().await;
    assert_eq!(observed.completion, TaskCompletion::RecoveryRequired);
    assert!(observed.diagnostic.is_none());
    assert!(transport.requests.lock().unwrap().is_empty());
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(submission.turn_id).unwrap();
    assert!(turn.attempts.is_empty());
    assert_eq!(turn.outcome, Some(TurnOutcome::Interrupted));
}

#[tokio::test]
async fn prefill_commit_failure_fences_real_agent_until_pair_reopens() {
    use crate::agent::repository::prefill::SqlitePrefillEstimator;
    use research_agent_core::context::accounting::ServerObservedEstimator;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.sqlite");
    let log = dir.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table prefill_fault_parent(id integer primary key); create table prefill_fault_child(id integer references prefill_fault_parent(id) deferrable initially deferred); create trigger prefill_fault after insert on agent_prefill_baselines begin insert into prefill_fault_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &log).unwrap();
    let app = crate::storage::AppStore::for_test_database(db.clone(), dir.path().join("objects"));
    let ((service, store, transport, _, mut execution, id), _) = setup_with_app(&app).await;
    let estimator = SqlitePrefillEstimator::new(
        db.clone(),
        id,
        &model(),
        Arc::new(ServerObservedEstimator::new(Arc::new(
            Utf8JsonTokenEstimator,
        ))),
    )
    .unwrap();
    execution.transport = Arc::new(Accounted {
        inner: transport.clone(),
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: model(),
            budget: None,
            estimator: Arc::new(estimator),
        }),
    });
    let submission = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
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
    let observed = owned.task.observe().await;
    assert_eq!(observed.completion, TaskCompletion::RecoveryRequired);
    let diagnostic = serde_json::to_value(observed.diagnostic.unwrap()).unwrap();
    assert_eq!(diagnostic["conversationId"], id.to_string());
    assert_eq!(diagnostic["phase"], "commit");
    assert_eq!(diagnostic["safeCause"], "sql_commit_outcome_unknown");
    assert_eq!(diagnostic["sqliteExtendedCode"], 787);
    assert!(transport.requests.lock().unwrap().is_empty());
    assert!(
        db.lock().is_err(),
        "uncertain commit must fence the data owner"
    );
    drop(service);
    drop(app);
    drop(db);

    let sql = Connection::open(&path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &log).unwrap();
    let app = crate::storage::AppStore::for_test_database(reopened, dir.path().join("objects"));
    let service = AgentSessionService::from_app_store(&app).unwrap();
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(submission.turn_id).unwrap();
    assert!(turn.attempts.is_empty());
    assert_eq!(turn.outcome, Some(TurnOutcome::Interrupted));
}

#[tokio::test]
async fn native_sampling_restores_and_persists_body_baseline_across_transport_recreation() {
    use crate::agent::repository::prefill::SqlitePrefillEstimator;
    use research_agent_core::context::accounting::ServerObservedEstimator;
    let ((service, store, transport, _, mut execution, id), db) = setup_with_connection().await;
    let mut events = response(false);
    events.insert(
        events.len() - 1,
        ModelStreamEvent::Usage(ModelUsage {
            input_tokens: Some(100),
            output_tokens: Some(20),
        }),
    );
    let mut second_events = response(false);
    second_events.insert(
        second_events.len() - 1,
        ModelStreamEvent::Usage(ModelUsage {
            input_tokens: Some(400),
            output_tokens: Some(20),
        }),
    );
    *transport.responses.lock().unwrap() = VecDeque::from(vec![events, second_events]);
    let baseline = Arc::new(
        SqlitePrefillEstimator::new(
            db.clone(),
            id,
            &model(),
            Arc::new(ServerObservedEstimator::new(Arc::new(
                Utf8JsonTokenEstimator,
            ))),
        )
        .unwrap(),
    );
    let policy = Arc::new(ContextAccounting {
        unknown_window_trigger_tokens: None,
        model: model(),
        budget: Some(CompactionBudget {
            context_window_tokens: 10000,
            effective_percent: 95,
            configured_trigger_tokens: Some(5000),
            output_reserve_tokens: 100,
        }),
        estimator: baseline,
    });
    execution.transport = Arc::new(Accounted {
        inner: transport.clone(),
        policy: policy.clone(),
    });
    let execution = Arc::new(execution);
    let first = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
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
    let restored = Arc::new(
        SqlitePrefillEstimator::new(
            db.clone(),
            id,
            &model(),
            Arc::new(ServerObservedEstimator::new(Arc::new(
                Utf8JsonTokenEstimator,
            ))),
        )
        .unwrap(),
    );
    let first_usage = service.context_usage(id, model()).await.unwrap();
    assert_eq!(
        first_usage.status,
        crate::agent::context_usage::WindowUsageStatus::Observed
    );
    assert_eq!(first_usage.current_tokens, Some(120));
    assert_eq!(
        first_usage.observation.unwrap().sample.budget,
        policy.budget
    );
    let next_transport = Arc::new(Accounted {
        inner: transport.clone(),
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: model(),
            budget: policy.budget,
            estimator: restored.clone(),
        }),
    });
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: None,
        model: execution.model.clone(),
        transport: next_transport,
        compaction_strategy: execution.compaction_strategy.clone(),
        host_tools: execution.host_tools.clone(),
        system_instructions: execution.system_instructions.clone(),
        limits: execution.limits,
        max_output_tokens: execution.max_output_tokens,
        max_input_bytes: execution.max_input_bytes,
    });
    let second = store
        .put(id, Uuid::new_v4(), input("second"))
        .await
        .unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store,
            id,
            second.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let second_usage = service.context_usage(id, model()).await.unwrap();
    assert_eq!(second_usage.current_tokens, Some(420));
    let mut changed_model = model();
    changed_model.revision = Some("other revision".into());
    assert_eq!(
        service
            .context_usage(id, changed_model)
            .await
            .unwrap()
            .status,
        crate::agent::context_usage::WindowUsageStatus::Unknown
    );
    assert_eq!(
        restored.auto_compaction_tokens(Some(420), None).unwrap(),
        Some(320)
    );
    let json: String = db
        .lock()
        .unwrap()
        .query_row(
            "select baseline_json from agent_prefill_baselines where conversation_id=?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let saved: research_agent_core::context::accounting::PrefillBaseline =
        serde_json::from_str(&json).unwrap();
    assert_eq!(saved.tokens, 100);
    assert!(saved.observed);
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

struct Accounted {
    inner: Arc<Transport>,
    policy: Arc<ContextAccounting>,
}
impl ModelTransport for Accounted {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn context_accounting(&self) -> Option<Arc<ContextAccounting>> {
        Some(self.policy.clone())
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}
fn account(execution: &mut AgentExecution, transport: Arc<Transport>) {
    execution.max_input_bytes = 131072; // Token trigger must work below the byte guard.
    execution.transport = Arc::new(Accounted {
        inner: transport,
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: model(),
            estimator: Arc::new(Utf8JsonTokenEstimator),
            budget: Some(CompactionBudget {
                // Leave room for the current canonical instructions/tool schemas
                // on turn one; accumulated evidence still triggers compaction.
                context_window_tokens: 1600,
                effective_percent: 95,
                configured_trigger_tokens: None,
                output_reserve_tokens: 100,
            }),
        }),
    });
}

fn account_with_low_trigger(execution: &mut AgentExecution, transport: Arc<Transport>) {
    account(execution, transport.clone());
    execution.transport = Arc::new(Accounted {
        inner: transport,
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: model(),
            estimator: Arc::new(Utf8JsonTokenEstimator),
            budget: Some(CompactionBudget {
                context_window_tokens: 1600,
                effective_percent: 95,
                configured_trigger_tokens: Some(600),
                output_reserve_tokens: 100,
            }),
        }),
    });
}

#[tokio::test]
async fn model_token_threshold_installs_replacement_below_byte_guard() {
    threshold_installs_replacement(false).await;
}

#[tokio::test]
async fn explicit_threshold_with_unknown_window_installs_replacement() {
    threshold_installs_replacement(true).await;
}

#[tokio::test]
async fn fresh_input_crossing_soft_threshold_does_not_compact_before_sampling() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false), response(false)]);
    account(&mut execution, transport.clone());
    execution.transport = Arc::new(Accounted {
        inner: transport.clone(),
        policy: Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: model(),
            estimator: Arc::new(Utf8JsonTokenEstimator),
            budget: Some(CompactionBudget {
                context_window_tokens: 2000,
                effective_percent: 95,
                configured_trigger_tokens: Some(1440),
                output_reserve_tokens: 100,
            }),
        }),
    });
    let execution = Arc::new(execution);
    let mut old = input("old");
    old.context[0].text = "old evidence ".repeat(160);
    let first = store.put(id, Uuid::new_v4(), old).await.unwrap();
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
    let mut fresh = input("fresh");
    fresh.context[0].text = "new evidence ".repeat(110);
    let second = store.put(id, Uuid::new_v4(), fresh).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store,
            id,
            second.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let accounting = execution.transport.context_accounting().unwrap();
    assert!(
        accounting
            .should_compact(&model(), &requests[1].messages, &requests[1].tools)
            .unwrap()
    );
}

async fn threshold_installs_replacement(unknown_window: bool) {
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(false), response(false), response(false)]);
    account_with_low_trigger(&mut execution, transport.clone());
    if unknown_window {
        execution.transport = Arc::new(Accounted {
            inner: transport.clone(),
            policy: Arc::new(ContextAccounting {
                model: model(),
                budget: None,
                unknown_window_trigger_tokens: Some(600),
                estimator: Arc::new(Utf8JsonTokenEstimator),
            }),
        });
    }
    let execution = Arc::new(execution);
    let mut old = input("old");
    old.context[0].text = "old evidence ".repeat(160);
    let first = store.put(id, Uuid::new_v4(), old).await.unwrap();
    let first_outcome = run(
        execution.clone(),
        service.clone(),
        store.clone(),
        id,
        first.request_id,
    )
    .await;
    assert_eq!(
        first_outcome,
        TaskCompletion::Completed,
        "{:?}",
        service
            .read_since(ConversationId::from_uuid(id), 0, 100)
            .await
            .unwrap()
            .items
            .iter()
            .map(|item| item.payload())
            .collect::<Vec<_>>()
    );
    let mut fresh = input("fresh");
    fresh.context[0].text = "new evidence ".repeat(10);
    let second = store.put(id, Uuid::new_v4(), fresh).await.unwrap();
    let result = run(
        execution.clone(),
        service.clone(),
        store,
        id,
        second.request_id,
    )
    .await;
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let events = service
        .read_since(ConversationId::from_uuid(id), 0, 100)
        .await
        .unwrap();
    assert_eq!(
        result,
        TaskCompletion::Completed,
        "{:?} {:?}",
        history.turn(second.turn_id),
        events
            .items
            .iter()
            .map(|event| event.payload())
            .collect::<Vec<_>>()
    );
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_some()
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[1].tools.is_empty());
    let accounting = execution.transport.context_accounting().unwrap();
    accounting
        .check(&model(), &requests[2].messages, &requests[2].tools)
        .unwrap();
    assert!(message_bytes(&requests[2].messages).unwrap() < execution.max_input_bytes);
}

#[tokio::test]
async fn oversized_summary_never_installs_or_sends_a_followup() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    let mut oversized = response(false);
    oversized[1] = ModelStreamEvent::TextDelta("summary ".repeat(500));
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false), oversized]);
    account_with_low_trigger(&mut execution, transport.clone());
    let execution = Arc::new(execution);
    let mut old = input("old");
    old.context[0].text = "old evidence ".repeat(160);
    let first = store.put(id, Uuid::new_v4(), old).await.unwrap();
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
    let mut fresh = input("fresh");
    fresh.context[0].text = "new evidence ".repeat(10);
    let second = store.put(id, Uuid::new_v4(), fresh).await.unwrap();
    assert_eq!(
        run(execution, service.clone(), store, id, second.request_id).await,
        TaskCompletion::Failed
    );
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    assert!(
        !service
            .open(ConversationId::from_uuid(id))
            .await
            .unwrap()
            .turn(first.turn_id)
            .unwrap()
            .items
            .is_empty()
    );
}

#[tokio::test]
async fn fresh_input_exceeding_effective_window_fails_before_summary_io() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    account(&mut execution, transport.clone());
    let mut fresh = input("fresh");
    fresh.context[0].text = "x".repeat(6000);
    let submission = store.put(id, Uuid::new_v4(), fresh).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            submission.request_id
        )
        .await,
        TaskCompletion::Failed
    );
    assert!(transport.requests.lock().unwrap().is_empty());
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(submission.turn_id).unwrap();
    assert_eq!(turn.attempts.len(), 1);
    assert_eq!(turn.outcome, Some(TurnOutcome::Failed));
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
}

struct LargeEvidence;
impl ToolExecutor for LargeEvidence {
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if call.name == "read" {
            Ok(())
        } else {
            Err(RunError::ToolRejected)
        }
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async {
            Ok(ToolOutput {
                content: "evidence ".repeat(700),
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn tool_followup_compacts_within_the_same_model_window_and_preserves_evidence() {
    let (service, store, transport, _, mut execution, id) = setup().await;
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(true), response(false), response(false)]);
    account(&mut execution, transport.clone());
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read authorized source".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        Arc::new(LargeEvidence),
    );
    let execution = Arc::new(execution);
    let submission = store.put(id, Uuid::new_v4(), input("fresh")).await.unwrap();
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store,
            id,
            submission.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].tools.is_empty());
        assert!(requests[1].messages.iter().any(|message| matches!(message,
            ModelMessage::ToolResult { content, .. } if content == &"evidence ".repeat(700))));
        execution
            .transport
            .context_accounting()
            .unwrap()
            .check(&model(), &requests[2].messages, &requests[2].tools)
            .unwrap();
    }
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_some()
    );
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    assert!(
        history
            .turn(submission.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| matches!(item.kind, ItemKind::ToolResult { .. })
                && item.text.contains("evidence"))
    );
}
