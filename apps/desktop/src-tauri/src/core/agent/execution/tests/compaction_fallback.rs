//! Supplier injection exercised through the real durable task owner.
use super::*;
use research_agent_core::context::compaction_strategy::*;

#[path = "compaction_fallback/cancel_validation.rs"]
mod cancel_validation;

struct Strategy {
    producer: ModelIdentity,
    calls: AtomicUsize,
    fails: bool,
    fallback: bool,
    pending: Option<Arc<Pending>>,
}
struct Pending {
    entered: tokio::sync::Notify,
    dropped: std::sync::atomic::AtomicBool,
}
struct PendingGuard(Arc<Pending>);
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
    }
}
impl CompactionStrategy for Strategy {
    fn binding(&self) -> CompactionBinding {
        CompactionBinding {
            model: model(),
            strategy_id: "fixture.compaction".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        }
    }
    fn execution_model(&self) -> ModelIdentity {
        self.producer.clone()
    }
    fn allows_current_model_fallback(&self, error: &ModelError) -> bool {
        self.fallback && matches!(error, ModelError::ContextWindowExceeded)
    }
    fn validate_output(&self, _: &CompactionOutput) -> Result<(), CompactionFailure> {
        Ok(())
    }
    fn compact(&self, _: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(pending) = &self.pending {
                let _guard = PendingGuard(pending.clone());
                pending.entered.notify_one();
                std::future::pending::<()>().await;
            }
            if self.fails {
                return Err(if self.producer != model() {
                    ModelError::ContextWindowExceeded
                } else {
                    ModelError::Authentication
                }
                .into());
            }
            Ok(CompactionCandidate {
                binding: self.binding(),
                output: CompactionOutput::TextSummary("retained evidence".into()),
                credential_scope_id: None,
                response_id: Some("fallback-response".into()),
                usage: ModelUsage {
                    input_tokens: Some(10),
                    output_tokens: Some(3),
                },
            })
        })
    }
}

#[tokio::test]
async fn fallback_records_distinct_producers_and_installs_only_success() {
    scenario(false, true, false).await;
}

#[tokio::test]
async fn remote_fallback_uses_each_producers_tool_view() {
    use research_agent_core::context::accounting::{ContextAccounting, Utf8JsonTokenEstimator};
    use research_agent_core::model::tool_output::{
        ToolOutputPolicy, project_tool_output, project_tool_results,
    };
    struct RemoteStrategy {
        producer: ModelIdentity,
        policy: Option<ToolOutputPolicy>,
        seen: Mutex<Vec<String>>,
        fail: bool,
    }
    impl CompactionStrategy for RemoteStrategy {
        fn binding(&self) -> CompactionBinding {
            CompactionBinding {
                model: model(),
                strategy_id: "fixture.remote".into(),
                state_schema: 1,
                kind: CompactionKind::RemoteOpaque,
            }
        }
        fn execution_model(&self) -> ModelIdentity {
            self.producer.clone()
        }
        fn tool_output_policy(&self) -> Option<ToolOutputPolicy> {
            self.policy
        }
        fn input_accounting(&self) -> Option<Arc<ContextAccounting>> {
            Some(Arc::new(ContextAccounting {
                model: self.producer.clone(),
                budget: None,
                unknown_window_trigger_tokens: None,
                estimator: Arc::new(Utf8JsonTokenEstimator),
            }))
        }
        fn retention_policy(&self) -> Option<CompactionRetention> {
            Some(CompactionRetention::RecentUserMessages)
        }
        fn allows_current_model_fallback(&self, error: &ModelError) -> bool {
            self.fail && matches!(error, ModelError::ContextWindowExceeded)
        }
        fn validate_output(&self, _: &CompactionOutput) -> Result<(), CompactionFailure> {
            Ok(())
        }
        fn compact(&self, request: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
            Box::pin(async move {
                let text = request
                    .source
                    .messages
                    .iter()
                    .find_map(|message| match message {
                        ModelMessage::ToolResult { content, .. } => Some(content.clone()),
                        _ => None,
                    })
                    .ok_or(CompactionFailure::InvalidOutput)?;
                self.seen.lock().unwrap().push(text);
                if self.fail {
                    return Err(ModelError::ContextWindowExceeded.into());
                }
                Ok(CompactionCandidate {
                    binding: self.binding(),
                    output: CompactionOutput::RemoteOpaque(
                        serde_json::json!({"type":"compaction","encrypted_content":"synthetic"}),
                    ),
                    credential_scope_id: Some(Uuid::from_u128(1)),
                    response_id: Some("remote-fallback".into()),
                    usage: ModelUsage::default(),
                })
            })
        }
    }
    struct ValidationTransport(Arc<Transport>);
    impl ModelTransport for ValidationTransport {
        fn supports_compaction_kind(&self, kind: CompactionKind) -> bool {
            kind == CompactionKind::RemoteOpaque
        }
        fn capabilities(&self) -> ModelCapabilities {
            self.0.capabilities()
        }
        fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
            self.0.start(request)
        }
        fn validate_compaction_window<'a>(
            &'a self,
            _: &'a research_agent_core::context::checkpoint::ContextCheckpoint,
            _: &'a ModelRequest,
        ) -> ModelFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }
    let (service, store, transport, _, mut execution, id) = setup().await;
    let first = store.put(id, Uuid::new_v4(), input("first")).await.unwrap();
    assert_eq!(
        run(
            Arc::new(AgentExecution {
                #[cfg(target_os = "macos")]
                stop_hooks: None,
                transport: transport.clone(),
                model: execution.model.clone(),
                compaction_strategy: execution.compaction_strategy.clone(),
                compaction_hooks: None,
                host_tools: execution.host_tools.clone(),
                system_instructions: execution.system_instructions.clone(),
                limits: execution.limits,
                max_output_tokens: execution.max_output_tokens,
                max_input_bytes: execution.max_input_bytes
            }),
            service.clone(),
            store.clone(),
            id,
            first.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    let history = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let mut source = research_agent_core::model::reconstruct_history(&history).unwrap();
    assert!(project_tool_results(
        &mut source,
        Some(ToolOutputPolicy::Tokens(2)),
        10000
    ));
    let full = "tool evidence";
    let expected_old = project_tool_output(full, Some(ToolOutputPolicy::Tokens(1)), 10000).unwrap();
    let expected_current =
        project_tool_output(full, Some(ToolOutputPolicy::Tokens(2)), 10000).unwrap();
    assert_ne!(expected_old, expected_current);
    let old = Arc::new(RemoteStrategy {
        producer: ModelIdentity {
            model_id: "old-producer".into(),
            ..model()
        },
        policy: Some(ToolOutputPolicy::Tokens(1)),
        seen: Mutex::new(vec![]),
        fail: true,
    });
    let current = Arc::new(RemoteStrategy {
        producer: model(),
        policy: Some(ToolOutputPolicy::Tokens(2)),
        seen: Mutex::new(vec![]),
        fail: false,
    });
    execution.compaction_strategy = Some(current.clone());
    execution.transport = Arc::new(ValidationTransport(transport));
    let record = store
        .put(id, Uuid::new_v4(), input("fallback"))
        .await
        .unwrap();
    let worker = service.clone();
    let selected = old.clone();
    let request = record.request_id;
    let admission = service
        .start_submission(
            store.as_ref(),
            ConversationId::from_uuid(id),
            request,
            move |record, history, signal| async move {
                let selected: Arc<dyn CompactionStrategy> = selected;
                let result = execution
                    .compact_history_selected(
                        &worker,
                        &record,
                        &history,
                        None,
                        CompactionPhase::PreTurn,
                        &signal,
                        &source,
                        &[],
                        &[],
                        &HashMap::new(),
                        Some(&selected),
                        false,
                    )
                    .await;
                assert!(result.is_ok(), "compaction failed: {:?}", result.err());
                ServiceTurnSink {
                    service: &worker,
                    conversation_id: ConversationId::from_uuid(id),
                    request_id: request,
                }
                .emit(EventPayload::TurnFinished {
                    turn_id: record.turn_id,
                    outcome: TurnOutcome::Completed,
                })
                .await?;
                Ok(())
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert_eq!(*old.seen.lock().unwrap(), vec![expected_old]);
    assert_eq!(*current.seen.lock().unwrap(), vec![expected_current]);
}
#[tokio::test]
async fn failed_fallback_returns_original_error_and_never_installs() {
    scenario(true, true, false).await;
}
#[tokio::test]
async fn supplier_without_permission_does_not_attempt_current_model() {
    scenario(true, false, false).await;
}
#[tokio::test]
async fn cancellation_drops_fallback_io_without_installing_a_window() {
    scenario(false, true, true).await;
}

async fn scenario(current_fails: bool, allow: bool, cancel: bool) {
    let (service, store, _, _, mut execution, id) = setup().await;
    let old_model = ModelIdentity {
        model_id: "old-producer".into(),
        ..model()
    };
    let old = Arc::new(Strategy {
        producer: old_model.clone(),
        calls: AtomicUsize::new(0),
        fails: true,
        fallback: allow,
        pending: None,
    });
    let pending = Arc::new(Pending {
        entered: tokio::sync::Notify::new(),
        dropped: std::sync::atomic::AtomicBool::new(false),
    });
    let current = Arc::new(Strategy {
        producer: model(),
        calls: AtomicUsize::new(0),
        fails: current_fails,
        fallback: false,
        pending: cancel.then_some(pending.clone()),
    });
    execution.compaction_strategy = Some(current.clone());
    let record = store
        .put(id, Uuid::new_v4(), input("fallback"))
        .await
        .unwrap();
    let worker = service.clone();
    let old_worker = old.clone();
    let turn = record.turn_id;
    let request = record.request_id;
    let admission = service
        .start_submission(
            store.as_ref(),
            ConversationId::from_uuid(id),
            request,
            move |record, _, signal| async move {
                let history = worker.open(ConversationId::from_uuid(id)).await.unwrap();
                let selected: Arc<dyn CompactionStrategy> = old_worker;
                let source = [ModelMessage::User {
                    content: vec![ContentPart::Text("old historical evidence".into())],
                    source_item_id: None,
                }];
                let result = execution
                    .compact_history_selected(
                        &worker,
                        &record,
                        &history,
                        None,
                        CompactionPhase::PreTurn,
                        &signal,
                        &source,
                        &[],
                        &[],
                        &HashMap::new(),
                        Some(&selected),
                        false,
                    )
                    .await;
                if cancel {
                    assert!(matches!(result, Err(RunError::Model(ModelError::Canceled))));
                    ServiceTurnSink {
                        service: &worker,
                        conversation_id: ConversationId::from_uuid(id),
                        request_id: request,
                    }
                    .emit(EventPayload::TurnFinished {
                        turn_id: turn,
                        outcome: TurnOutcome::Canceled,
                    })
                    .await?;
                    return Err(ModelError::Canceled.into());
                }
                if current_fails || !allow {
                    assert!(matches!(
                        result,
                        Err(RunError::Model(ModelError::ContextWindowExceeded))
                    ));
                    assert!(
                        worker
                            .active_context_checkpoint(id)
                            .await
                            .unwrap()
                            .is_none()
                    );
                } else {
                    result.unwrap();
                    assert!(
                        worker
                            .active_context_checkpoint(id)
                            .await
                            .unwrap()
                            .is_some()
                    );
                }
                ServiceTurnSink {
                    service: &worker,
                    conversation_id: ConversationId::from_uuid(id),
                    request_id: request,
                }
                .emit(EventPayload::TurnFinished {
                    turn_id: turn,
                    outcome: if current_fails || !allow {
                        TurnOutcome::Failed
                    } else {
                        TurnOutcome::Completed
                    },
                })
                .await?;
                if current_fails || !allow {
                    Err(ModelError::ContextWindowExceeded.into())
                } else {
                    Ok(())
                }
            },
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    if cancel {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            pending.entered.notified(),
        )
        .await
        .unwrap();
        assert!(
            service
                .tasks()
                .cancel_turn(ConversationId::from_uuid(id), owned.turn_id)
                .unwrap()
        );
        assert_eq!(owned.task.wait().await, TaskCompletion::Canceled);
        assert!(pending.dropped.load(Ordering::SeqCst));
        assert!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .is_none()
        );
    } else {
        assert_eq!(
            owned.task.wait().await,
            if current_fails || !allow {
                TaskCompletion::Failed
            } else {
                TaskCompletion::Completed
            }
        );
    }
    assert_eq!(old.calls.load(Ordering::SeqCst), 1);
    assert_eq!(current.calls.load(Ordering::SeqCst), usize::from(allow));
    let projection = service.open(ConversationId::from_uuid(id)).await.unwrap();
    let saved = projection.turn(turn).unwrap();
    let compact_items: Vec<_> = saved
        .items
        .values()
        .filter(|item| item.kind == research_agent_core::protocol::ItemKind::ContextCompaction)
        .collect();
    assert_eq!(compact_items.len(), 1);
    let item = compact_items[0];
    assert_eq!(
        item.kind,
        research_agent_core::protocol::ItemKind::ContextCompaction
    );
    assert_eq!(item.completed, !current_fails && !cancel && allow);
    assert_eq!(item.interrupted, !item.completed);
    let attempts: Vec<_> = saved
        .attempt_order
        .iter()
        .map(|id| (*id, saved.attempts[id].requested_model.clone()))
        .collect();
    assert_eq!(attempts.len(), if allow { 2 } else { 1 });
    assert_eq!(attempts[0].1, old_model);
    if allow {
        assert_ne!(attempts[0].0, attempts[1].0);
        assert_eq!(attempts[1].1, model());
        if !current_fails && !cancel {
            assert_eq!(saved.attempts[&attempts[1].0].actual_model, Some(model()));
            let window = service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .unwrap();
            assert!(saved.items[&window.window_id].completed);
            assert!(attempts.iter().all(|(id, _)| *id != window.window_id));
        }
    }
}
