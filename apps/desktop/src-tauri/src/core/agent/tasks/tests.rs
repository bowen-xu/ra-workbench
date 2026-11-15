use super::*;
use tokio::sync::oneshot;

fn conversation() -> ConversationId {
    ConversationId::from_uuid(Uuid::new_v4())
}

#[tokio::test]
async fn turn_lookup_does_not_identify_an_unadmitted_checker_as_owner() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let turn = Uuid::new_v4();
    let claimed = Arc::new(AtomicBool::new(false));
    let (release, waiting) = oneshot::channel();
    let task = registry
        .start_with_cleanup_claim(
            id,
            Some(turn),
            claimed.clone(),
            |_| async move {
                waiting.await.unwrap();
                Ok(())
            },
            |result| async move { result },
        )
        .unwrap();
    assert!(registry.find_turn_receipt(id, turn).unwrap().is_none());
    claimed.store(true, Ordering::Release);
    assert_eq!(
        registry
            .find_turn_receipt(id, turn)
            .unwrap()
            .unwrap()
            .run_id(),
        task.run_id()
    );
    release.send(()).unwrap();
    assert_eq!(task.wait().await, TaskCompletion::Completed);
}

#[tokio::test]
async fn proven_turn_discovers_only_its_unambiguous_original_owner() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let turn = Uuid::new_v4();
    let original = registry
        .start_with_cleanup_claim(
            id,
            Some(turn),
            Arc::new(AtomicBool::new(true)),
            |_| async { Ok(()) },
            |result| async move { result },
        )
        .unwrap();
    assert_eq!(
        registry
            .find_turn_receipt(id, turn)
            .unwrap()
            .unwrap()
            .run_id(),
        original.run_id()
    );
    assert!(
        registry
            .find_turn_receipt(conversation(), turn)
            .unwrap()
            .is_none()
    );
    assert!(
        registry
            .find_turn_receipt(id, Uuid::new_v4())
            .unwrap()
            .is_none()
    );
    assert_eq!(original.wait().await, TaskCompletion::Completed);
    let later = registry
        .start_with_cleanup_claim(
            id,
            Some(Uuid::new_v4()),
            Arc::new(AtomicBool::new(true)),
            |_| async { Ok(()) },
            |result| async move { result },
        )
        .unwrap();
    assert_eq!(later.wait().await, TaskCompletion::Completed);
    assert_eq!(
        registry
            .find_turn_receipt(id, turn)
            .unwrap()
            .unwrap()
            .run_id(),
        original.run_id()
    );
    let duplicate = registry
        .start_with_cleanup_claim(
            id,
            Some(turn),
            Arc::new(AtomicBool::new(true)),
            |_| async { Ok(()) },
            |result| async move { result },
        )
        .unwrap();
    assert!(registry.find_turn_receipt(id, turn).unwrap().is_none());
    assert_eq!(duplicate.wait().await, TaskCompletion::Completed);
}

#[tokio::test]
async fn exact_receipt_survives_later_admission_and_idle_reservation() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let turn = Uuid::new_v4();
    let old = registry
        .start_with_cleanup(
            id,
            Some(turn),
            move |_| async move {
                Err(RunError::Model(ModelError::PersistenceDiagnostic(
                    research_agent_core::model::ContextPersistenceDiagnostic {
                        conversation_id: *id.as_uuid(),
                        phase: "write",
                        safe_cause: "sql_write_failed",
                        sqlite_extended_code: Some(1811),
                    },
                )))
            },
            |result| async move { result },
        )
        .unwrap();
    assert_eq!(old.wait().await, TaskCompletion::RecoveryRequired);
    let old_run = old.run_id();
    drop(old);
    let reservation = registry.reserve_idle(conversation()).unwrap();
    drop(reservation);
    let newer = registry.start(id, |_| async { Ok(()) }).unwrap();
    assert_eq!(newer.wait().await, TaskCompletion::Completed);
    let old = registry.find_receipt(id, old_run).unwrap().unwrap();
    assert_eq!(old.turn_id(), Some(turn));
    let observation = old.observe().await;
    assert_eq!(observation.completion, TaskCompletion::RecoveryRequired);
    let diagnostic = serde_json::to_value(observation.diagnostic.unwrap()).unwrap();
    assert_eq!(diagnostic["sqliteExtendedCode"], 1811);
    assert_eq!(diagnostic["conversationId"], id.as_uuid().to_string());
    assert!(
        registry
            .find_receipt(conversation(), old_run)
            .unwrap()
            .is_none()
    );
    assert!(registry.find_receipt(id, Uuid::new_v4()).unwrap().is_none());
}

#[tokio::test]
async fn recent_receipts_are_bounded_and_eviction_is_not_a_completion_claim() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let first = registry.start(id, |_| async { Ok(()) }).unwrap();
    let first_id = first.run_id();
    assert_eq!(first.wait().await, TaskCompletion::Completed);
    for _ in 0..=MAX_RECENT_RECEIPTS {
        let receipt = registry.start(id, |_| async { Ok(()) }).unwrap();
        assert_eq!(receipt.wait().await, TaskCompletion::Completed);
    }
    assert!(registry.find_receipt(id, first_id).unwrap().is_none());
    assert_eq!(
        registry.state.lock().unwrap().recent.len(),
        MAX_RECENT_RECEIPTS
    );
    // An existing direct receipt continues to own its result after eviction.
    assert_eq!(first.wait().await, TaskCompletion::Completed);
}

#[tokio::test]
async fn exact_receipt_lookup_reports_pending_without_waiting_for_the_owner() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let (release, waiting) = oneshot::channel();
    let original = registry
        .start(id, |_| async move {
            waiting.await.unwrap();
            Ok(())
        })
        .unwrap();
    let found = registry
        .find_receipt(id, original.run_id())
        .unwrap()
        .unwrap();
    assert!(!found.is_finished());
    release.send(()).unwrap();
    assert_eq!(found.wait().await, TaskCompletion::Completed);
    assert!(found.is_finished());
}

#[tokio::test]
async fn scoped_drain_after_an_admission_fence_preserves_other_task_owners() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let owned = registry
        .start(id, |signal| async move {
            signal.cancelled().await;
            Err(RunError::Model(
                research_agent_core::model::ModelError::Canceled,
            ))
        })
        .unwrap();
    let other_id = conversation();
    let (release, gate) = oneshot::channel();
    let other = registry
        .start(other_id, |signal| async move {
            gate.await.unwrap();
            assert!(!signal.is_cancelled());
            Ok(())
        })
        .unwrap();
    // The fixture adapter has fenced new admission for id before this call.
    assert_eq!(
        registry.cancel_and_wait_scope(&[id]).await.unwrap(),
        vec![(id, TaskCompletion::Canceled)]
    );
    assert_eq!(owned.wait().await, TaskCompletion::Canceled);
    assert!(registry.has_active(other_id).unwrap());
    let fresh = registry
        .start(conversation(), |_| async { Ok(()) })
        .unwrap();
    assert_eq!(fresh.wait().await, TaskCompletion::Completed);
    release.send(()).unwrap();
    assert_eq!(other.wait().await, TaskCompletion::Completed);
}

#[tokio::test]
async fn one_task_per_conversation_and_stale_stop_is_fenced() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let (release, gate) = oneshot::channel();
    let first = registry
        .start(id, |signal| async move {
            gate.await.unwrap();
            assert!(signal.is_cancelled());
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    assert!(matches!(
        registry.start(id, |_| async { Ok(()) }),
        Err(TaskRegistryError::Active)
    ));
    assert!(!registry.cancel(id, Uuid::new_v4()).unwrap());
    assert!(registry.cancel(id, first.run_id()).unwrap());
    release.send(()).unwrap();
    assert_eq!(first.wait().await, TaskCompletion::Canceled);
    let (release, gate) = oneshot::channel();
    let next = registry
        .start(id, |signal| async move {
            gate.await.unwrap();
            assert!(!signal.is_cancelled());
            Ok(())
        })
        .unwrap();
    assert_ne!(first.run_id(), next.run_id());
    assert!(!registry.cancel(id, first.run_id()).unwrap());
    release.send(()).unwrap();
    assert_eq!(next.wait().await, TaskCompletion::Completed);
    assert_eq!(first.wait().await, TaskCompletion::Canceled);
}

#[tokio::test]
async fn dropping_receipt_keeps_task_owned_and_other_conversations_can_run() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let (release, gate) = oneshot::channel();
    let (done, completed) = oneshot::channel();
    let receipt = registry
        .start(id, |signal| async move {
            gate.await.unwrap();
            assert!(!signal.is_cancelled());
            done.send(()).unwrap();
            Ok(())
        })
        .unwrap();
    drop(receipt);
    assert!(matches!(
        registry.start(id, |_| async { Ok(()) }),
        Err(TaskRegistryError::Active)
    ));
    let other = registry
        .start(conversation(), |_| async { Ok(()) })
        .unwrap();
    assert_eq!(other.wait().await, TaskCompletion::Completed);
    release.send(()).unwrap();
    completed.await.unwrap();
    registry.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_waiter_can_be_canceled_without_losing_task_or_reopening_admission() {
    let registry = AgentTaskRegistry::default();
    let (release, gate) = oneshot::channel();
    let receipt = registry
        .start(conversation(), |signal| async move {
            gate.await.unwrap(); // Represents a durable commit that must finish.
            assert!(signal.is_cancelled());
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    let mut shutdown = Box::pin(registry.shutdown());
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(shutdown.as_mut().poll(cx).is_pending()))
            .await
    );
    drop(shutdown);
    assert!(matches!(
        registry.start(conversation(), |_| async { Ok(()) }),
        Err(TaskRegistryError::Closing)
    ));
    release.send(()).unwrap();
    let results = registry.shutdown().await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1, TaskCompletion::Canceled);
    assert_eq!(receipt.wait().await, TaskCompletion::Canceled);
}

#[tokio::test]
async fn persistence_failure_and_panic_require_reconciliation() {
    let registry = AgentTaskRegistry::default();
    let failed = registry
        .start(conversation(), |_| async { Err(RunError::EventSink) })
        .unwrap();
    assert_eq!(failed.wait().await, TaskCompletion::RecoveryRequired);
    let detail =
        research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(serde_json::json!({
            "code":"TOOL_RECEIPT_UNCONFIRMED", "phase":"result-ack", "safeCause":"owner_ack_failed"
        }))
        .unwrap();
    let detailed = registry
        .start(conversation(), |_| async {
            Err(RunError::EventSinkDiagnostic(Box::new(detail)))
        })
        .unwrap();
    assert_eq!(detailed.wait().await, TaskCompletion::RecoveryRequired);
    let observed = detailed.observe().await;
    assert_eq!(observed.completion, TaskCompletion::RecoveryRequired);
    assert_eq!(
        serde_json::to_value(observed.diagnostic.unwrap()).unwrap()["code"],
        "TOOL_RECEIPT_UNCONFIRMED"
    );
    let accounting = registry
        .start(conversation(), |_| async {
            Err(RunError::Model(ModelError::PersistenceDiagnostic(
                research_agent_core::model::ContextPersistenceDiagnostic {
                    conversation_id: Uuid::from_u128(1),
                    phase: "commit",
                    safe_cause: "sql_commit_outcome_unknown",
                    sqlite_extended_code: Some(787),
                },
            )))
        })
        .unwrap();
    assert_eq!(accounting.wait().await, TaskCompletion::RecoveryRequired);
    assert_eq!(
        serde_json::to_value(accounting.observe().await.diagnostic.unwrap()).unwrap()["sqliteExtendedCode"],
        787
    );
    let unknown = registry
        .start(conversation(), |_| async {
            Err(RunError::ToolOutcomeUnknown)
        })
        .unwrap();
    assert_eq!(unknown.wait().await, TaskCompletion::RecoveryRequired);
    let panicked = registry
        .start(conversation(), |_| async { panic!("synthetic task panic") })
        .unwrap();
    assert_eq!(panicked.wait().await, TaskCompletion::RecoveryRequired);
    let rejected = registry
        .start(conversation(), |_| async { Err(RunError::ToolRejected) })
        .unwrap();
    assert_eq!(rejected.wait().await, TaskCompletion::Failed);
    assert_eq!(
        serde_json::to_value(detailed.observe().await.diagnostic.unwrap()).unwrap()["phase"],
        "result-ack",
        "a later task must not replace the original receipt's diagnosis"
    );
    registry.shutdown().await.unwrap();
}

#[tokio::test]
async fn structured_unknown_reaches_the_original_supervisor_cleanup_as_recovery_required() {
    let registry = AgentTaskRegistry::default();
    let conversation = conversation();
    let turn = Uuid::new_v4();
    let request = Uuid::new_v4();
    let detail=research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(serde_json::json!({
        "code":"TEXT_PATCH_JOURNAL_UNCONFIRMED","conversationId":conversation.as_uuid(),"requestId":request,"turnId":turn,
        "phase":"call-terminal","safeCause":"journal_sql_failed","sqliteCode":"ConstraintViolation","sqliteExtendedCode":1811,
    })).unwrap();
    let (send, receive) = oneshot::channel();
    let receipt = registry
        .start_with_cleanup(
            conversation,
            Some(turn),
            move |_| async move { Err(RunError::ToolOutcomeUnknownDiagnostic(Box::new(detail))) },
            move |completion| async move {
                send.send(completion).unwrap();
                completion
            },
        )
        .unwrap();
    assert_eq!(receive.await.unwrap(), TaskCompletion::RecoveryRequired);
    assert_eq!(receipt.wait().await, TaskCompletion::RecoveryRequired);
    assert_eq!(receipt.wait().await, TaskCompletion::RecoveryRequired);
    let rejected = registry
        .start(conversation, |_| async { Err(RunError::ToolRejected) })
        .unwrap();
    assert_eq!(rejected.wait().await, TaskCompletion::Failed);
    registry.shutdown().await.unwrap();
}

#[tokio::test]
async fn cleanup_panic_keeps_original_diagnostic_and_recovery_status() {
    let registry = AgentTaskRegistry::default();
    let detail = research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(
        serde_json::json!({"code":"context_persistence_failure", "phase":"commit", "safeCause":"sql_commit_outcome_unknown"}),
    ).unwrap();
    let receipt = registry
        .start_with_cleanup(
            conversation(),
            Some(Uuid::new_v4()),
            move |_| async move { Err(RunError::EventSinkDiagnostic(Box::new(detail))) },
            |_| async move { panic!("synthetic cleanup panic") },
        )
        .unwrap();
    let observed = receipt.observe().await;
    assert_eq!(observed.completion, TaskCompletion::RecoveryRequired);
    assert_eq!(
        serde_json::to_value(observed.diagnostic.unwrap()).unwrap()["code"],
        "context_persistence_failure"
    );
    registry.shutdown().await.unwrap();
}

#[test]
fn missing_runtime_returns_error_without_poisoning_registry() {
    let registry = AgentTaskRegistry::default();
    assert!(matches!(
        registry.start(conversation(), |_| async { Ok(()) }),
        Err(TaskRegistryError::Unavailable)
    ));
    assert!(registry.state.lock().is_ok());
}

#[tokio::test]
async fn cleanup_keeps_admission_reserved_until_reconciliation_finishes() {
    let registry = AgentTaskRegistry::default();
    let id = conversation();
    let (entered, ready) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let receipt = registry
        .start_with_cleanup(
            id,
            None,
            |_| async { Ok(()) },
            |result| async move {
                entered.send(()).unwrap();
                gate.await.unwrap();
                result
            },
        )
        .unwrap();
    ready.await.unwrap();
    assert!(matches!(
        registry.start(id, |_| async { Ok(()) }),
        Err(TaskRegistryError::Active)
    ));
    registry.cancel(id, receipt.run_id()).unwrap();
    release.send(()).unwrap();
    assert_eq!(receipt.wait().await, TaskCompletion::Completed);
}

#[tokio::test]
async fn parent_cancellation_propagates_to_bounded_child() {
    let registry = AgentTaskRegistry::default();
    let parent_id = conversation();
    let child_id = conversation();
    let context = Uuid::new_v4();
    let (parent_release, parent_gate) = oneshot::channel();
    let parent = registry
        .start(parent_id, |signal| async move {
            parent_gate.await.unwrap();
            assert!(signal.is_cancelled());
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    let parent_spec = research_agent_core::subtasks::SubtaskSpec {
        parent_conversation_id: *parent_id.as_uuid(),
        child_conversation_id: *parent_id.as_uuid(),
        parent_context_snapshot_id: context,
        child_context_snapshot_id: context,
        depth: 0,
        model_calls: 4,
        tool_calls: 4,
        approved_servers: Default::default(),
    };
    let child_spec = research_agent_core::subtasks::SubtaskSpec {
        parent_conversation_id: *parent_id.as_uuid(),
        child_conversation_id: *child_id.as_uuid(),
        parent_context_snapshot_id: context,
        child_context_snapshot_id: context,
        depth: 1,
        model_calls: 1,
        tool_calls: 1,
        approved_servers: Default::default(),
    };
    registry.cancel(parent_id, parent.run_id()).unwrap();
    let child = registry
        .start_child(&parent_spec, &child_spec, |signal| async move {
            signal.cancelled().await;
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    parent_release.send(()).unwrap();
    assert_eq!(parent.wait().await, TaskCompletion::Canceled);
    assert_eq!(child.wait().await, TaskCompletion::Canceled);
}

#[tokio::test]
async fn linking_already_started_child_preserves_parent_cancellation() {
    let registry = AgentTaskRegistry::default();
    let parent_id = conversation();
    let child_id = conversation();
    let (parent_release, parent_gate) = oneshot::channel();
    let parent = registry
        .start(parent_id, |signal| async move {
            parent_gate.await.unwrap();
            assert!(signal.is_cancelled());
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    let child = registry
        .start(child_id, |signal| async move {
            signal.cancelled().await;
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    assert!(registry.has_active(parent_id).unwrap());
    registry.link_child(parent_id, child_id).unwrap();
    registry.cancel(parent_id, parent.run_id()).unwrap();
    parent_release.send(()).unwrap();
    assert_eq!(parent.wait().await, TaskCompletion::Canceled);
    assert_eq!(child.wait().await, TaskCompletion::Canceled);
    assert!(matches!(
        registry.link_child(parent_id, parent_id),
        Err(TaskRegistryError::SubtaskInvalid)
    ));
    assert!(matches!(
        registry.link_child(child_id, parent_id),
        Err(TaskRegistryError::SubtaskInvalid)
    ));
}

#[tokio::test]
async fn linking_after_parent_cancel_propagates_to_already_started_child() {
    let registry = AgentTaskRegistry::default();
    let parent_id = conversation();
    let child_id = conversation();
    let (release, gate) = oneshot::channel();
    let parent = registry
        .start(parent_id, |signal| async move {
            gate.await.unwrap();
            assert!(signal.is_cancelled());
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    let child = registry
        .start(child_id, |signal| async move {
            signal.cancelled().await;
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    assert!(registry.cancel(parent_id, parent.run_id()).unwrap());
    registry.link_child(parent_id, child_id).unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    release.send(()).unwrap();
    assert_eq!(parent.wait().await, TaskCompletion::Canceled);
}

#[tokio::test]
async fn completion_remains_visible_while_join_is_locked() {
    let registry = AgentTaskRegistry::default();
    let parent_id = conversation();
    let child_id = conversation();
    let (parent_release, parent_gate) = oneshot::channel();
    let parent = registry
        .start(parent_id, |_| async move {
            parent_gate.await.unwrap();
            Ok(())
        })
        .unwrap();
    let (child_release, child_gate) = oneshot::channel();
    let child = registry
        .start(child_id, |_| async move {
            child_gate.await.unwrap();
            Ok(())
        })
        .unwrap();
    let join_guard = parent.0.join.lock().await;
    parent_release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !parent.0.completion.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!registry.has_active(parent_id).unwrap());
    assert!(matches!(
        registry.link_child(parent_id, child_id),
        Err(TaskRegistryError::SubtaskInvalid)
    ));
    let replacement = registry.start(parent_id, |_| async { Ok(()) }).unwrap();
    drop(join_guard);
    assert_eq!(parent.wait().await, TaskCompletion::Completed);
    assert_eq!(replacement.wait().await, TaskCompletion::Completed);
    child_release.send(()).unwrap();
    assert_eq!(child.wait().await, TaskCompletion::Completed);
}

#[tokio::test]
async fn linking_to_finished_parent_is_rejected() {
    let registry = AgentTaskRegistry::default();
    let parent_id = conversation();
    let child_id = conversation();
    let (release, gate) = oneshot::channel();
    let parent = registry
        .start(parent_id, |_| async move {
            gate.await.unwrap();
            Ok(())
        })
        .unwrap();
    let (child_release, child_gate) = oneshot::channel();
    let child = registry
        .start(child_id, |_| async move {
            child_gate.await.unwrap();
            Ok(())
        })
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(parent.wait().await, TaskCompletion::Completed);
    assert!(matches!(
        registry.link_child(parent_id, child_id),
        Err(TaskRegistryError::SubtaskInvalid)
    ));
    child_release.send(()).unwrap();
    assert_eq!(child.wait().await, TaskCompletion::Completed);
}

#[tokio::test]
async fn parent_cancellation_propagates_through_two_level_child_lineage() {
    let registry = AgentTaskRegistry::default();
    let parent_id = conversation();
    let child_id = conversation();
    let grandchild_id = conversation();
    let context = Uuid::new_v4();
    let parent_turn_id = Uuid::new_v4();
    let (parent_release, parent_gate) = oneshot::channel();
    let parent = registry
        .start_with_cleanup(
            parent_id,
            Some(parent_turn_id),
            |signal| async move {
                parent_gate.await.unwrap();
                assert!(signal.is_cancelled());
                Err(ModelError::Canceled.into())
            },
            |result| async move { result },
        )
        .unwrap();
    let parent_spec = research_agent_core::subtasks::SubtaskSpec {
        parent_conversation_id: *parent_id.as_uuid(),
        child_conversation_id: *parent_id.as_uuid(),
        parent_context_snapshot_id: context,
        child_context_snapshot_id: context,
        depth: 0,
        model_calls: 4,
        tool_calls: 4,
        approved_servers: Default::default(),
    };
    let child_spec = research_agent_core::subtasks::SubtaskSpec {
        parent_conversation_id: *parent_id.as_uuid(),
        child_conversation_id: *child_id.as_uuid(),
        parent_context_snapshot_id: context,
        child_context_snapshot_id: Uuid::new_v4(),
        depth: 1,
        model_calls: 2,
        tool_calls: 2,
        approved_servers: Default::default(),
    };
    let grandchild_spec = research_agent_core::subtasks::SubtaskSpec {
        parent_conversation_id: *child_id.as_uuid(),
        child_conversation_id: *grandchild_id.as_uuid(),
        parent_context_snapshot_id: child_spec.child_context_snapshot_id,
        child_context_snapshot_id: Uuid::new_v4(),
        depth: 2,
        model_calls: 1,
        tool_calls: 1,
        approved_servers: Default::default(),
    };
    let child = registry
        .start_child(&parent_spec, &child_spec, |signal| async move {
            signal.cancelled().await;
            Err(ModelError::Canceled.into())
        })
        .unwrap();
    let grandchild = registry
        .start_child(&child_spec, &grandchild_spec, |signal| async move {
            signal.cancelled().await;
            Err(ModelError::Canceled.into())
        })
        .unwrap();

    assert!(registry.cancel_turn(parent_id, parent_turn_id).unwrap());
    parent_release.send(()).unwrap();
    assert_eq!(parent.wait().await, TaskCompletion::Canceled);
    assert_eq!(child.wait().await, TaskCompletion::Canceled);
    assert_eq!(grandchild.wait().await, TaskCompletion::Canceled);
}

#[tokio::test]
async fn completed_lineage_is_pruned_before_new_admission() {
    let registry = AgentTaskRegistry::default();
    let parent_id = conversation();
    let child_id = conversation();
    let (parent_release, parent_gate) = oneshot::channel();
    let (child_release, child_gate) = oneshot::channel();
    let parent = registry
        .start(parent_id, |_| async move {
            parent_gate.await.unwrap();
            Ok(())
        })
        .unwrap();
    let child = registry
        .start(child_id, |_| async move {
            child_gate.await.unwrap();
            Ok(())
        })
        .unwrap();
    registry.link_child(parent_id, child_id).unwrap();
    parent_release.send(()).unwrap();
    child_release.send(()).unwrap();
    assert_eq!(parent.wait().await, TaskCompletion::Completed);
    assert_eq!(child.wait().await, TaskCompletion::Completed);

    let next = registry
        .start(conversation(), |_| async { Ok(()) })
        .unwrap();
    assert!(
        registry
            .state
            .lock()
            .expect("registry state")
            .children
            .is_empty()
    );
    assert_eq!(next.wait().await, TaskCompletion::Completed);
}
