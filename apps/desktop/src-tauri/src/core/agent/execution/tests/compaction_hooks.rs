use super::*;
use research_agent_core::context::compaction_hooks::*;

struct Hooks {
    stop_at: CompactionHookStage,
    outcome: CompactionHookOutcome,
    events: Mutex<Vec<(CompactionHookRequest, CompactionHookNotification)>>,
}
struct QuietHandlerObserver;
impl HookHandlerObserver for QuietHandlerObserver {
    fn notify(&self, _: Uuid, _: &CompactionHookRequest, _: CompactionHookNotification) {}
}

struct SelectedHook {
    phase: CompactionPhase,
    allowed: bool,
    calls: AtomicUsize,
}
impl CompactionHooks for SelectedHook {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn accepts_request(&self, request: &CompactionHookRequest) -> bool {
        request.phase == self.phase
    }
    fn validate_request(&self, _: &CompactionHookRequest) -> Result<(), HookContextError> {
        if self.allowed {
            Ok(())
        } else {
            Err(HookContextError)
        }
    }
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { CompactionHookOutcome::Continue })
    }
}

#[tokio::test]
async fn manual_compaction_admits_only_matching_authorized_handlers() {
    for allow_manual in [true, false] {
        let ((service, store, transport, _, execution, id), db) = setup_with_connection().await;
        *transport.responses.lock().unwrap() =
            VecDeque::from(vec![response(false), response(false)]);
        let execution = Arc::new(execution);
        let old = store
            .put(id, Uuid::new_v4(), input("old-source"))
            .await
            .unwrap();
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
        let mut execution = Arc::try_unwrap(execution).ok().unwrap();
        let manual = Arc::new(SelectedHook {
            phase: CompactionPhase::Manual,
            allowed: allow_manual,
            calls: AtomicUsize::new(0),
        });
        let automatic = Arc::new(SelectedHook {
            phase: CompactionPhase::MidTurn,
            allowed: false,
            calls: AtomicUsize::new(0),
        });
        execution.compaction_hooks = Some(Arc::new(
            ConcurrentCompactionHooks::new(
                vec![
                    CompactionHookHandler {
                        id: Uuid::from_u128(10),
                        stage: CompactionHookStage::PreCompact,
                        hooks: manual.clone(),
                    },
                    CompactionHookHandler {
                        id: Uuid::from_u128(11),
                        stage: CompactionHookStage::PreCompact,
                        hooks: automatic.clone(),
                    },
                ],
                Arc::new(QuietHandlerObserver),
            )
            .unwrap(),
        ));
        let (_, admission) = compact(Arc::new(execution), service, store, id).await;
        assert_eq!(
            completed(admission).await,
            if allow_manual {
                TaskCompletion::Completed
            } else {
                TaskCompletion::Failed
            }
        );
        let db = db.lock().unwrap();
        let handlers: Vec<String> = db
            .prepare("select handler_id from agent_compaction_hook_handlers order by audit_seq")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            handlers,
            if allow_manual {
                vec![
                    Uuid::from_u128(10).to_string(),
                    Uuid::from_u128(10).to_string(),
                ]
            } else {
                vec![]
            }
        );
        assert_eq!(
            manual.calls.load(Ordering::SeqCst),
            usize::from(allow_manual)
        );
        assert_eq!(automatic.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            transport.requests.lock().unwrap().len(),
            if allow_manual { 2 } else { 1 }
        );
    }
}
impl CompactionHooks for Hooks {
    fn notify(&self, request: &CompactionHookRequest, notification: CompactionHookNotification) {
        self.events
            .lock()
            .unwrap()
            .push((request.clone(), notification));
    }
    fn run(&self, request: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async move {
            if request.stage == self.stop_at {
                self.outcome
            } else {
                CompactionHookOutcome::Continue
            }
        })
    }
}

async fn verify_boundary(stage: CompactionHookStage, outcome: CompactionHookOutcome) {
    let ((service, store, transport, _, mut execution, id), db) = setup_with_connection().await;
    *transport.responses.lock().unwrap() =
        VecDeque::from(vec![response(false), response(false), response(false)]);
    let old = store
        .put(id, Uuid::new_v4(), input("old-source"))
        .await
        .unwrap();
    let execution_owner = Arc::new(execution);
    assert_eq!(
        run(
            execution_owner.clone(),
            service.clone(),
            store.clone(),
            id,
            old.request_id
        )
        .await,
        TaskCompletion::Completed
    );
    execution = Arc::try_unwrap(execution_owner)
        .ok()
        .expect("finished task released execution owner");
    let hooks = Arc::new(Hooks {
        stop_at: stage,
        outcome,
        events: Mutex::new(vec![]),
    });
    execution.compaction_hooks = Some(hooks.clone());
    let execution = Arc::new(execution);
    let (record, admitted) = compact(execution.clone(), service.clone(), store.clone(), id).await;
    let stopped = outcome == CompactionHookOutcome::Stop;
    assert_eq!(
        completed(admitted).await,
        if stopped {
            TaskCompletion::Canceled
        } else {
            TaskCompletion::Completed
        }
    );
    let checkpoint = service.active_context_checkpoint(id).await.unwrap();
    let pre_stop = stopped && stage == CompactionHookStage::PreCompact;
    assert_eq!(checkpoint.is_none(), pre_stop);
    if pre_stop {
        assert!(
            service
                .open(ConversationId::from_uuid(id))
                .await
                .unwrap()
                .turn(record.turn_id)
                .unwrap()
                .attempts
                .is_empty()
        );
    }
    assert_eq!(
        transport.requests.lock().unwrap().len(),
        if pre_stop { 1 } else { 2 }
    );
    let events = hooks.events.lock().unwrap().clone();
    assert_eq!(events.len(), if pre_stop { 2 } else { 4 });
    assert_eq!(events[0].1, CompactionHookNotification::Started);
    assert_eq!(events[0].0.conversation_id, id);
    assert_eq!(events[0].0.turn_id, record.turn_id);
    assert_eq!(events[0].0.phase, CompactionPhase::Manual);
    assert_eq!(events[0].0.model, model());
    assert!(
        events
            .iter()
            .all(|event| event.0.operation_id == events[0].0.operation_id)
    );
    let statuses: Vec<(String, Option<String>)> = db
        .lock()
        .unwrap()
        .prepare("select notification,outcome from agent_compaction_hook_audit order by seq")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(statuses.len(), events.len());
    assert_eq!(statuses[0], ("started".into(), None));
    assert_eq!(
        statuses[1],
        (
            "completed".into(),
            Some(
                if pre_stop {
                    "stop"
                } else if outcome == CompactionHookOutcome::Failed {
                    "failed"
                } else {
                    "continue"
                }
                .into()
            )
        )
    );
    if stage == CompactionHookStage::PostCompact {
        assert_eq!(events[3].1, CompactionHookNotification::Completed(outcome));
    }
    if !pre_stop {
        let installed = checkpoint.unwrap();
        // Post stop preserves installation and the next regular turn can use it.
        let next = store
            .put(id, Uuid::new_v4(), input("after-hook"))
            .await
            .unwrap();
        assert_eq!(
            run(execution, service.clone(), store, id, next.request_id).await,
            TaskCompletion::Completed
        );
        assert_eq!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .unwrap()
                .window_id,
            installed.window_id
        );
        assert!(
            message_text(&transport.requests.lock().unwrap()[2].messages)
                .contains("Context checkpoint handoff")
        );
    }
}

#[tokio::test]
async fn pre_compact_stop_prevents_model_request_and_installation() {
    verify_boundary(CompactionHookStage::PreCompact, CompactionHookOutcome::Stop).await;
}

#[tokio::test]
async fn post_compact_stop_preserves_installed_window_for_next_turn() {
    verify_boundary(
        CompactionHookStage::PostCompact,
        CompactionHookOutcome::Stop,
    )
    .await;
}

#[tokio::test]
async fn failed_pre_compact_hook_does_not_veto_compaction() {
    verify_boundary(
        CompactionHookStage::PreCompact,
        CompactionHookOutcome::Failed,
    )
    .await;
}

struct PendingHook {
    entered: tokio::sync::Notify,
    dropped: Arc<std::sync::atomic::AtomicBool>,
    events: Mutex<Vec<CompactionHookNotification>>,
}
struct DropHook(Arc<std::sync::atomic::AtomicBool>);
impl Drop for DropHook {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl CompactionHooks for PendingHook {
    fn notify(&self, _: &CompactionHookRequest, notification: CompactionHookNotification) {
        self.events.lock().unwrap().push(notification);
    }
    fn run(&self, _: CompactionHookRequest) -> CompactionHookFuture<'_> {
        Box::pin(async move {
            let _guard = DropHook(self.dropped.clone());
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}

#[tokio::test]
async fn cancellation_drops_pending_hook_before_any_model_request() {
    let ((service, store, transport, _, execution, id), db) = setup_with_connection().await;
    *transport.responses.lock().unwrap() = VecDeque::from(vec![response(false)]);
    let execution = Arc::new(execution);
    let old = store
        .put(id, Uuid::new_v4(), input("old-source"))
        .await
        .unwrap();
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
    let mut execution = Arc::try_unwrap(execution).ok().unwrap();
    let hooks = Arc::new(PendingHook {
        entered: tokio::sync::Notify::new(),
        dropped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        events: Mutex::new(vec![]),
    });
    let unmatched = Arc::new(SelectedHook {
        phase: CompactionPhase::MidTurn,
        allowed: false,
        calls: AtomicUsize::new(0),
    });
    execution.compaction_hooks = Some(Arc::new(
        ConcurrentCompactionHooks::new(
            vec![
                CompactionHookHandler {
                    id: Uuid::from_u128(10),
                    stage: CompactionHookStage::PreCompact,
                    hooks: hooks.clone(),
                },
                CompactionHookHandler {
                    id: Uuid::from_u128(11),
                    stage: CompactionHookStage::PreCompact,
                    hooks: unmatched.clone(),
                },
            ],
            Arc::new(QuietHandlerObserver),
        )
        .unwrap(),
    ));
    let (_, admission) = compact(Arc::new(execution), service.clone(), store, id).await;
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("owned task expected")
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), hooks.entered.notified())
        .await
        .unwrap();
    service
        .tasks()
        .cancel_turn(ConversationId::from_uuid(id), owned.turn_id)
        .unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(2), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    let completed: String = db
        .lock()
        .unwrap()
        .query_row(
            "select outcome from agent_compaction_hook_audit where notification='completed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(completed, "canceled");
    let handler_states: Vec<Option<String>> = db
        .lock()
        .unwrap()
        .prepare("select outcome from agent_compaction_hook_handlers order by audit_seq")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(handler_states, [None, Some("canceled".into())]);
    assert_eq!(unmatched.calls.load(Ordering::SeqCst), 0);
    assert!(hooks.dropped.load(Ordering::SeqCst));
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert!(
        service
            .active_context_checkpoint(id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        *hooks.events.lock().unwrap(),
        vec![
            CompactionHookNotification::Started,
            CompactionHookNotification::Completed(CompactionHookOutcome::Canceled)
        ]
    );
}

#[tokio::test]
async fn compaction_hook_audit_storage_failure_blocks_execution_without_undoing_post_install() {
    for (stage, notification) in [
        ("pre_compact", "started"),
        ("pre_compact", "completed"),
        ("post_compact", "completed"),
    ] {
        let ((service, store, transport, _, execution, id), db) = setup_with_connection().await;
        *transport.responses.lock().unwrap() =
            VecDeque::from(vec![response(false), response(false)]);
        let owner = Arc::new(execution);
        let old = store
            .put(id, Uuid::new_v4(), input("old-source"))
            .await
            .unwrap();
        assert_eq!(
            run(
                owner.clone(),
                service.clone(),
                store.clone(),
                id,
                old.request_id
            )
            .await,
            TaskCompletion::Completed
        );
        let mut execution = Arc::try_unwrap(owner).ok().unwrap();
        let hooks = Arc::new(Hooks {
            stop_at: CompactionHookStage::PreCompact,
            outcome: CompactionHookOutcome::Continue,
            events: Mutex::new(vec![]),
        });
        execution.compaction_hooks = Some(hooks.clone());
        db.lock().unwrap().execute_batch(&format!("create trigger reject_audit before insert on agent_compaction_hook_audit when new.stage='{stage}' and new.notification='{notification}' begin select raise(abort,'synthetic storage failure'); end;")).unwrap();
        let (_, admitted) = compact(Arc::new(execution), service.clone(), store, id).await;
        assert_eq!(completed(admitted).await, TaskCompletion::RecoveryRequired);
        let post = stage == "post_compact";
        assert_eq!(
            service
                .active_context_checkpoint(id)
                .await
                .unwrap()
                .is_some(),
            post
        );
        assert_eq!(
            transport.requests.lock().unwrap().len(),
            if post { 2 } else { 1 }
        );
        assert_eq!(
            hooks.events.lock().unwrap().len(),
            if notification == "started" {
                0
            } else if post {
                3
            } else {
                1
            }
        );
        let completed: i64 = db
            .lock()
            .unwrap()
            .query_row(
                "select count(*) from agent_compaction_hook_audit where notification='completed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(completed, if post { 1 } else { 0 });
    }
}
