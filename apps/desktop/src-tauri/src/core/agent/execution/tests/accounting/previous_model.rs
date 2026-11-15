use super::*;
use crate::agent::execution_profile::model_transition::PreviousModelStrategy;
use research_agent_core::context::{
    compaction_hooks::{
        CompactionHookFuture, CompactionHookNotification, CompactionHookOutcome,
        CompactionHookRequest, CompactionHookResult, CompactionHookResultFuture,
        CompactionHookStage, CompactionHooks,
    },
    compaction_strategy::{CompactionBinding, CompactionKind, CompactionStrategy},
    text_compaction::TextCompactionStrategy,
};

struct OrderedHooks(Mutex<Vec<CompactionHookStage>>);
impl CompactionHooks for OrderedHooks {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, request: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request.stage);
            CompactionHookOutcome::Continue
        })
    }
    fn run_with_context(&self, request: CompactionHookRequest) -> CompactionHookResultFuture<'_> {
        Box::pin(async move {
            let stage = request.stage;
            let outcome = self.run(request).await;
            CompactionHookResult {
                outcome,
                additional_contexts: if stage == CompactionHookStage::SessionStartCompact {
                    vec!["queued hook context".into()]
                } else {
                    vec![]
                },
            }
        })
    }
}

struct TransitionTransport {
    inner: Arc<Transport>,
    strategy: Arc<dyn CompactionStrategy>,
    previous_turn: Uuid,
    tool_policy: Option<research_agent_core::model::tool_output::ToolOutputPolicy>,
    accounting: Option<Arc<ContextAccounting>>,
}
impl ModelTransport for TransitionTransport {
    fn context_accounting(&self) -> Option<Arc<ContextAccounting>> {
        self.accounting.clone()
    }
    fn tool_output_policy(
        &self,
    ) -> Option<research_agent_core::model::tool_output::ToolOutputPolicy> {
        self.tool_policy
    }
    fn pre_turn_compaction<'a>(
        &'a self,
        turn: Uuid,
        previous: &'a ModelIdentity,
        _: Option<u64>,
    ) -> ModelFuture<'a, Option<Arc<dyn CompactionStrategy>>> {
        assert_eq!(turn, self.previous_turn);
        assert_eq!(*previous, model());
        Box::pin(async { Ok(Some(self.strategy.clone())) })
    }
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}

#[tokio::test]
async fn old_model_summary_is_validated_for_the_current_model_and_records_its_producer() {
    transition(false, false).await;
}
#[tokio::test]
async fn old_model_failure_stops_current_sampling_without_installing_a_candidate() {
    transition(true, false).await;
}
#[tokio::test]
async fn nested_fork_sends_inherited_history_to_the_old_producer_before_current_sampling() {
    transition(false, true).await;
}

#[tokio::test]
async fn previous_model_compaction_uses_its_frozen_tool_view() {
    producer_tool_view(false).await;
}

#[tokio::test]
async fn previous_turn_of_same_model_uses_its_frozen_tool_view() {
    producer_tool_view(true).await;
}

#[tokio::test]
async fn previous_model_compaction_is_followed_by_current_budget_check() {
    double_compaction(700, false, false).await;
}

#[tokio::test]
async fn previous_model_compaction_below_current_threshold_does_not_repeat() {
    double_compaction(500, false, false).await;
}

#[tokio::test]
async fn failed_second_compaction_preserves_first_checkpoint_and_stops_sampling() {
    double_compaction(700, true, false).await;
}

#[tokio::test]
async fn old_candidate_over_current_hard_limit_is_compacted_before_sampling() {
    double_compaction(1000, false, true).await;
}

#[tokio::test]
async fn over_limit_intermediate_survives_failed_second_compaction() {
    double_compaction(1000, true, true).await;
}

async fn double_compaction(tokens: u64, fail_second: bool, over_hard: bool) {
    struct FixedUsage(u64, bool);
    impl ContextTokenEstimator for FixedUsage {
        fn estimate(&self, messages: &[ModelMessage], _: &[ToolDefinition]) -> Option<u64> {
            Some(if self.1 && !text(messages).contains("OLD_OVERSIZED") {
                500
            } else {
                self.0
            })
        }
    }
    let root = tempfile::tempdir().unwrap();
    let db_path = root.path().join("handoff.sqlite3");
    let mut connection = Connection::open(&db_path).unwrap();
    crate::storage::migrate(&mut connection).unwrap();
    let app = crate::storage::AppStore::for_test(connection, root.path().join("objects"));
    let ((service, store, old_wire, _, mut execution, id), db) = setup_with_app(&app).await;
    let mut old_summary = response(false);
    if over_hard {
        old_summary[1] = ModelStreamEvent::TextDelta("OLD_OVERSIZED".into());
    }
    *old_wire.responses.lock().unwrap() = VecDeque::from(vec![response(false), old_summary]);
    let first = store.put(id, Uuid::new_v4(), input("old")).await.unwrap();
    let first_execution = Arc::new(execution);
    assert_eq!(
        run(
            first_execution.clone(),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    execution = Arc::try_unwrap(first_execution).ok().unwrap();
    let hooks = Arc::new(OrderedHooks(Mutex::new(vec![])));
    execution.compaction_hooks = Some(hooks.clone());
    let target = ModelIdentity {
        provider_id: "new-provider".into(),
        model_id: "new-model".into(),
        revision: None,
    };
    let binding = CompactionBinding {
        model: target.clone(),
        strategy_id: "research.text.v1".into(),
        state_schema: 1,
        kind: CompactionKind::TextSummary,
    };
    let selected = Arc::new(
        PreviousModelStrategy::new(
            text_strategy(old_wire.clone()),
            old_wire.clone(),
            binding.clone(),
            None,
            64,
        )
        .unwrap(),
    );
    let mut current_response = response(false);
    if let ModelStreamEvent::Started { actual_model, .. } = &mut current_response[0] {
        *actual_model = target.clone();
    }
    let current = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(if fail_second {
            VecDeque::from(vec![vec![]])
        } else {
            VecDeque::from(vec![current_response.clone(), current_response])
        }),
    });
    execution.model = target.clone();
    execution.compaction_strategy = Some(Arc::new(
        TextCompactionStrategy::new(current.clone(), binding).unwrap(),
    ));
    execution.transport = Arc::new(TransitionTransport {
        inner: current.clone(),
        strategy: selected,
        previous_turn: first.turn_id,
        tool_policy: None,
        accounting: Some(Arc::new(ContextAccounting {
            unknown_window_trigger_tokens: None,
            model: target.clone(),
            estimator: Arc::new(FixedUsage(tokens, over_hard)),
            budget: Some(CompactionBudget {
                context_window_tokens: 1000,
                effective_percent: 95,
                configured_trigger_tokens: Some(600),
                output_reserve_tokens: 100,
            }),
        })),
    });
    let mut fresh = input("fresh");
    fresh.model = target.clone();
    fresh.text = "unique fresh input".into();
    let second = store.put(id, Uuid::new_v4(), fresh).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store.clone(),
            id,
            second.request_id
        )
        .await,
        if fail_second {
            TaskCompletion::Failed
        } else {
            TaskCompletion::Completed
        }
    );
    assert_eq!(old_wire.requests.lock().unwrap().len(), 2);
    let second_compaction = tokens >= 600;
    {
        let requests = current.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            if second_compaction && !fail_second {
                2
            } else {
                1
            }
        );
        if second_compaction {
            assert!(requests[0].tools.is_empty());
            assert!(!text(&requests[0].messages).contains("unique fresh input"));
            assert!(!text(&requests[0].messages).contains("queued hook context"));
            if over_hard {
                assert!(text(&requests[0].messages).contains("OLD_OVERSIZED"));
            }
        }
        if !fail_second {
            assert!(text(&requests[requests.len() - 1].messages).contains("unique fresh input"));
            assert_eq!(requests[requests.len() - 1].messages.iter().filter(|message| matches!(message, ModelMessage::Developer(body) if body == "queued hook context")).count(), if second_compaction { 2 } else { 1 });
        }
        let stages = hooks.0.lock().unwrap().clone();
        assert_eq!(
            stages,
            if fail_second {
                vec![
                    CompactionHookStage::PreCompact,
                    CompactionHookStage::PostCompact,
                    CompactionHookStage::PreCompact,
                ]
            } else if second_compaction {
                vec![
                    CompactionHookStage::PreCompact,
                    CompactionHookStage::PostCompact,
                    CompactionHookStage::PreCompact,
                    CompactionHookStage::PostCompact,
                    CompactionHookStage::SessionStartCompact,
                    CompactionHookStage::SessionStartCompact,
                ]
            } else {
                vec![
                    CompactionHookStage::PreCompact,
                    CompactionHookStage::PostCompact,
                    CompactionHookStage::SessionStartCompact,
                ]
            }
        );
    }
    let window = service
        .active_context_checkpoint(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        window.previous_window_id.is_some(),
        second_compaction && !fail_second
    );
    if over_hard && fail_second {
        let saved_window = window.window_id;
        let connection = Arc::downgrade(&db);
        drop(db);
        drop(service);
        drop(store);
        drop(old_wire);
        drop(current);
        drop(app);
        assert!(
            connection.upgrade().is_none(),
            "the original database owner must close"
        );

        let mut db = Connection::open(&db_path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let app = crate::storage::AppStore::for_test(db, root.path().join("objects"));
        let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
        let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
        service.open(ConversationId::from_uuid(id)).await.unwrap();
        assert_eq!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .unwrap()
                .window_id,
            saved_window
        );
        let (_, _, transport, _, mut resumed, _) = setup().await;
        let mut response = response(false);
        if let ModelStreamEvent::Started { actual_model, .. } = &mut response[0] {
            *actual_model = target.clone();
        }
        *transport.responses.lock().unwrap() = VecDeque::from(vec![response.clone(), response]);
        resumed.model = target.clone();
        resumed.compaction_strategy = Some(Arc::new(
            TextCompactionStrategy::new(
                transport.clone(),
                CompactionBinding {
                    model: target.clone(),
                    strategy_id: "research.text.v1".into(),
                    state_schema: 1,
                    kind: CompactionKind::TextSummary,
                },
            )
            .unwrap(),
        ));
        resumed.transport = Arc::new(Accounted {
            inner: transport.clone(),
            policy: Arc::new(ContextAccounting {
                unknown_window_trigger_tokens: None,
                model: target.clone(),
                estimator: Arc::new(FixedUsage(1000, true)),
                budget: Some(CompactionBudget {
                    context_window_tokens: 1000,
                    effective_percent: 95,
                    configured_trigger_tokens: Some(600),
                    output_reserve_tokens: 100,
                }),
            }),
        });
        let mut retry = input("retry");
        retry.model = target;
        retry.text = "after cold reopen".into();
        let retry = store.put(id, Uuid::new_v4(), retry).await.unwrap();
        assert_eq!(
            run(
                Arc::new(resumed),
                service.clone(),
                store,
                id,
                retry.request_id
            )
            .await,
            TaskCompletion::Completed
        );
        {
            let requests = transport.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests[0].tools.is_empty());
            assert!(text(&requests[0].messages).contains("OLD_OVERSIZED"));
            assert!(!text(&requests[0].messages).contains("after cold reopen"));
            assert!(!text(&requests[1].messages).contains("OLD_OVERSIZED"));
            assert!(text(&requests[1].messages).contains("after cold reopen"));
        }
        assert_eq!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .unwrap()
                .previous_window_id,
            Some(saved_window)
        );
    }
}

async fn producer_tool_view(same_model: bool) {
    use research_agent_core::model::tool_output::{ToolOutputPolicy, project_tool_output};
    struct PolicyTransport(Arc<Transport>);
    impl ModelTransport for PolicyTransport {
        fn capabilities(&self) -> ModelCapabilities {
            self.0.capabilities()
        }
        fn tool_output_policy(&self) -> Option<ToolOutputPolicy> {
            Some(ToolOutputPolicy::Tokens(25))
        }
        fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
            self.0.start(request)
        }
    }
    struct LargeTool;
    impl ToolExecutor for LargeTool {
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
    let (service, store, old_wire, _, mut execution, id) = setup().await;
    *old_wire.responses.lock().unwrap() =
        VecDeque::from(vec![response(true), response(false), response(false)]);
    let old_policy: Arc<dyn ModelTransport> = Arc::new(PolicyTransport(old_wire.clone()));
    execution.transport = old_policy.clone();
    execution.host_tools = research_agent_core::tool_registry::OwnedToolRegistration::new(
        vec![ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read".into(),
            parameters: serde_json::json!({"type":"object"}),
        }],
        Arc::new(LargeTool),
    );
    let old = store
        .put(id, Uuid::new_v4(), input("old source"))
        .await
        .unwrap();
    let mut execution = Arc::new(execution);
    assert_eq!(
        run(
            execution.clone(),
            service.clone(),
            store.clone(),
            id,
            old.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let old_view =
        project_tool_output(&"x".repeat(8000), Some(ToolOutputPolicy::Tokens(25)), 10000).unwrap();
    let new_view =
        project_tool_output(&"x".repeat(8000), Some(ToolOutputPolicy::Tokens(5)), 10000).unwrap();
    assert_ne!(old_view, new_view);
    let target = if same_model {
        model()
    } else {
        ModelIdentity {
            provider_id: "new-provider".into(),
            model_id: "new-model".into(),
            revision: None,
        }
    };
    let binding = CompactionBinding {
        model: target.clone(),
        strategy_id: "research.text.v1".into(),
        state_schema: 1,
        kind: CompactionKind::TextSummary,
    };
    let selected = Arc::new(
        PreviousModelStrategy::new(
            text_strategy(old_policy.clone()),
            old_policy,
            binding.clone(),
            None,
            64,
        )
        .unwrap(),
    );
    let mut answer = response(false);
    if let ModelStreamEvent::Started { actual_model, .. } = &mut answer[0] {
        *actual_model = target.clone();
    }
    let current = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from(vec![answer])),
    });
    let active = Arc::get_mut(&mut execution).unwrap();
    active.model = target.clone();
    active.compaction_strategy = Some(Arc::new(
        TextCompactionStrategy::new(current.clone(), binding).unwrap(),
    ));
    active.transport = Arc::new(TransitionTransport {
        inner: current,
        strategy: selected,
        previous_turn: old.turn_id,
        tool_policy: Some(ToolOutputPolicy::Tokens(5)),
        accounting: None,
    });
    let mut next = input("new source");
    next.model = target;
    let next = store.put(id, Uuid::new_v4(), next).await.unwrap();
    assert_eq!(
        run(execution, service, store, id, next.request_id).await,
        TaskCompletion::Completed
    );
    let requests = old_wire.requests.lock().unwrap();
    let summary = &requests[2];
    assert!(summary.messages.iter().any(|message| matches!(message, ModelMessage::ToolResult { content, .. } if content == &old_view)));
    assert!(!summary.messages.iter().any(|message| matches!(message, ModelMessage::ToolResult { content, .. } if content == &new_view)));
}
async fn transition(fail: bool, fork: bool) {
    let ((service, store, old_transport, tools, execution, mut id), db) =
        setup_with_connection().await;
    *old_transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let old = store
        .put(id, Uuid::new_v4(), input("covered historical source"))
        .await
        .unwrap();
    let first_execution = Arc::new(execution);
    assert_eq!(
        run(
            first_execution.clone(),
            service.clone(),
            store.clone(),
            id,
            old.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let mut execution = Arc::try_unwrap(first_execution).ok().unwrap();
    if fork {
        use crate::agent::repository::SqliteConversationCatalog;
        use research_agent_core::conversation::*;
        {
            let db = db.lock().unwrap();
            db.execute("insert into note_pages(id,title,created_at,updated_at) values ('old-model-note','Note','now','now')", []).unwrap();
            db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','old-model-note')", [id.to_string()]).unwrap();
        }
        let catalog = SqliteConversationCatalog::new(db);
        for _ in 0..2 {
            let child = Uuid::new_v4();
            catalog
                .fork_legacy_for_tests(ConversationForkRequest {
                    child_id: ConversationId::from_uuid(child),
                    parent_id: ConversationId::from_uuid(id),
                    through_turn_id: old.turn_id,
                    binding: ConversationBinding {
                        kind: BindingKind::Note,
                        resource_id: "old-model-note".into(),
                    },
                })
                .await
                .unwrap();
            id = child;
        }
        service.open(ConversationId::from_uuid(id)).await.unwrap();
    }
    let target_model = ModelIdentity {
        provider_id: "new-provider".into(),
        model_id: "new-model".into(),
        revision: None,
    };
    let binding = CompactionBinding {
        model: target_model.clone(),
        strategy_id: "research.text.v1".into(),
        state_schema: 1,
        kind: CompactionKind::TextSummary,
    };
    let selected = Arc::new(
        PreviousModelStrategy::new(
            text_strategy(old_transport.clone()),
            old_transport.clone(),
            binding.clone(),
            None,
            64,
        )
        .unwrap(),
    );
    if fail {
        old_transport.responses.lock().unwrap().push_back(vec![]);
    } else {
        let mut summary = response(false);
        summary[1] =
            ModelStreamEvent::TextDelta("previous-model handoff with historical evidence".into());
        old_transport.responses.lock().unwrap().push_back(summary);
    }
    let mut next_events = response(false);
    if let ModelStreamEvent::Started { actual_model, .. } = &mut next_events[0] {
        *actual_model = target_model.clone();
    }
    let current = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(VecDeque::from(vec![next_events])),
    });
    execution.model = target_model.clone();
    execution.system_instructions = "current authoritative rules".into();
    execution.compaction_strategy = Some(Arc::new(
        TextCompactionStrategy::new(current.clone(), binding).unwrap(),
    ));
    execution.transport = Arc::new(TransitionTransport {
        inner: current.clone(),
        strategy: selected,
        previous_turn: old.turn_id,
        tool_policy: None,
        accounting: None,
    });
    let mut next = input("fresh-current-source");
    next.model = target_model.clone();
    next.text = "fresh-current-input".into();
    let next = store.put(id, Uuid::new_v4(), next).await.unwrap();
    assert_eq!(
        run(
            Arc::new(execution),
            service.clone(),
            store,
            id,
            next.request_id
        )
        .await,
        if fail {
            TaskCompletion::Failed
        } else {
            TaskCompletion::Completed
        }
    );
    assert_eq!(old_transport.requests.lock().unwrap().len(), 2);
    let summary = old_transport.requests.lock().unwrap()[1].clone();
    assert_eq!(summary.model, model());
    assert!(summary.tools.is_empty());
    assert!(summary.max_output_tokens <= 64);
    let old_text = text(&summary.messages);
    assert!(old_text.contains("covered historical source"));
    assert!(!old_text.contains("fresh-current-input"));
    assert!(!old_text.contains("fresh-current-source"));
    assert_eq!(tools.0.load(Ordering::SeqCst), 0);
    let checkpoint = service.active_context_checkpoint(id).await.unwrap();
    if fail {
        assert!(checkpoint.is_none());
        assert!(current.requests.lock().unwrap().is_empty());
    } else {
        assert_eq!(checkpoint.unwrap().binding.model, target_model);
        let requests = current.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model, target_model);
        let current_text = text(&requests[0].messages);
        assert!(current_text.contains("previous-model handoff"));
        assert!(current_text.contains("fresh-current-input"));
        assert!(current_text.contains("current authoritative rules"));
        assert!(!current_text.contains("covered historical source"));
    }
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let turn = history.turn(next.turn_id).unwrap();
    assert_eq!(turn.model, target_model);
    assert!(
        turn.attempts
            .values()
            .any(|attempt| attempt.requested_model == model())
    );
    if !fail {
        assert!(
            turn.attempts
                .values()
                .any(|attempt| attempt.actual_model == Some(model()))
        );
    }
}
fn text(messages: &[ModelMessage]) -> String {
    messages
        .iter()
        .flat_map(|message| match message {
            ModelMessage::CanonicalOutput(item) => match &item.normalized {
                Some(research_agent_core::model::ModelOutputItem::Message(message)) => {
                    vec![message.text.as_str()]
                }
                _ => vec![],
            },
            ModelMessage::System(text)
            | ModelMessage::Developer(text)
            | ModelMessage::ToolResult { content: text, .. } => {
                vec![text.as_str()]
            }
            ModelMessage::User { content: parts, .. }
            | ModelMessage::Assistant { content: parts, .. } => parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
