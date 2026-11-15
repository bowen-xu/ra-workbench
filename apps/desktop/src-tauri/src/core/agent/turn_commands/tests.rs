use super::*;
use crate::agent::tasks::TaskCompletion;
use research_agent_core::model::ModelError;
use tokio::sync::oneshot;

#[tokio::test]
async fn observe_exact_task_never_infers_an_evicted_or_different_identity() {
    let registry = AgentTaskRegistry::default();
    let id = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let (release, gate) = oneshot::channel();
    let receipt = registry
        .start_with_cleanup(
            ConversationId::from_uuid(id),
            Some(turn),
            move |_| async move {
                gate.await.unwrap();
                Err(ModelError::PersistenceDiagnostic(
                    research_agent_core::model::ContextPersistenceDiagnostic {
                        conversation_id: id,
                        phase: "write",
                        safe_cause: "sql_write_failed",
                        sqlite_extended_code: Some(1811),
                    },
                )
                .into())
            },
            |result| async move { result },
        )
        .unwrap();
    let run = receipt.run_id();
    let request = || TaskObservationRequest {
        conversation_id: id,
        turn_id: turn,
        run_id: run,
    };
    let pending = observe_task(&registry, request()).await.unwrap();
    assert_eq!(
        serde_json::to_value(pending).unwrap()["status"]["type"],
        "pending"
    );
    let wrong = observe_task(
        &registry,
        TaskObservationRequest {
            turn_id: Uuid::new_v4(),
            ..request()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(wrong).unwrap()["status"]["type"],
        "unavailable"
    );
    release.send(()).unwrap();
    assert_eq!(receipt.wait().await, TaskCompletion::RecoveryRequired);
    let next = registry
        .start(ConversationId::from_uuid(id), |_| async { Ok(()) })
        .unwrap();
    assert_eq!(next.wait().await, TaskCompletion::Completed);
    let finished = serde_json::to_value(observe_task(&registry, request()).await.unwrap()).unwrap();
    assert_eq!(finished["conversationId"], id.to_string());
    assert_eq!(finished["turnId"], turn.to_string());
    assert_eq!(finished["runId"], run.to_string());
    assert_eq!(finished["status"]["completion"], "recoveryRequired");
    assert_eq!(finished["status"]["diagnostic"]["sqliteExtendedCode"], 1811);
    assert!(
        observe_task(
            &registry,
            TaskObservationRequest {
                run_id: Uuid::nil(),
                ..request()
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn repeated_stop_signals_only_matching_turn_and_does_not_claim_completion() {
    let fixtures: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../../../../tests/fixtures/agent-interrupt-v1.json"
    ))
    .unwrap();
    let id = Uuid::parse_str(fixtures[0]["conversationId"].as_str().unwrap()).unwrap();
    let turn = Uuid::parse_str(fixtures[0]["turnId"].as_str().unwrap()).unwrap();
    let registry = AgentTaskRegistry::default();
    let (release, gate) = oneshot::channel();
    let receipt = registry
        .start_with_cleanup(
            ConversationId::from_uuid(id),
            Some(turn),
            |signal| async move {
                gate.await.unwrap();
                assert!(signal.is_cancelled());
                Err(ModelError::Canceled.into())
            },
            |result| async move { result },
        )
        .unwrap();
    for request in [
        InterruptRequest {
            conversation_id: Uuid::new_v4(),
            turn_id: turn,
        },
        InterruptRequest {
            conversation_id: id,
            turn_id: Uuid::new_v4(),
        },
    ] {
        assert!(
            !interrupt(&registry, request)
                .unwrap()
                .cancellation_requested
        );
    }
    assert!(
        finish_interrupt(
            &registry,
            InterruptRequest {
                conversation_id: id,
                turn_id: turn
            },
            Err(crate::agent::repository::input_queue::Error::Storage)
        )
        .is_err()
    );
    for _ in 0..2 {
        let response = interrupt(
            &registry,
            InterruptRequest {
                conversation_id: id,
                turn_id: turn,
            },
        )
        .unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), fixtures[0]);
    }
    // The durable owner is deliberately still waiting after the acknowledgement.
    assert!(matches!(
        registry.start(ConversationId::from_uuid(id), |_| async { Ok(()) }),
        Err(crate::agent::tasks::TaskRegistryError::Active)
    ));
    release.send(()).unwrap();
    assert_eq!(receipt.wait().await, TaskCompletion::Canceled);
    let response = interrupt(
        &registry,
        InterruptRequest {
            conversation_id: id,
            turn_id: turn,
        },
    )
    .unwrap();
    assert_eq!(serde_json::to_value(response).unwrap(), fixtures[1]);
    let (release, gate) = oneshot::channel();
    let next = registry
        .start_with_cleanup(
            ConversationId::from_uuid(id),
            Some(Uuid::new_v4()),
            |signal| async move {
                gate.await.unwrap();
                assert!(!signal.is_cancelled());
                Ok(())
            },
            |result| async move { result },
        )
        .unwrap();
    assert!(
        !interrupt(
            &registry,
            InterruptRequest {
                conversation_id: id,
                turn_id: turn
            }
        )
        .unwrap()
        .cancellation_requested
    );
    release.send(()).unwrap();
    assert_eq!(next.wait().await, TaskCompletion::Completed);
}

#[test]
fn interrupt_request_rejects_unknown_fields_and_invalid_identities() {
    for json in [
        r#"{"conversationId":"bad","turnId":"bad"}"#,
        r#"{"conversationId":"11111111-1111-4111-8111-111111111111","turnId":"22222222-2222-4222-8222-222222222222","command":"kill"}"#,
    ] {
        assert!(serde_json::from_str::<InterruptRequest>(json).is_err());
    }
}

#[tokio::test]
async fn queue_storage_failure_still_signals_cancellation() {
    let registry = AgentTaskRegistry::default();
    let id = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let (release, gate) = oneshot::channel();
    let receipt = registry
        .start_with_cleanup(
            ConversationId::from_uuid(id),
            Some(turn),
            |signal| async move {
                gate.await.unwrap();
                assert!(signal.is_cancelled());
                Err(ModelError::Canceled.into())
            },
            |result| async move { result },
        )
        .unwrap();
    assert!(
        finish_interrupt(
            &registry,
            InterruptRequest {
                conversation_id: id,
                turn_id: turn
            },
            Err(crate::agent::repository::input_queue::Error::Storage)
        )
        .is_err()
    );
    release.send(()).unwrap();
    assert_eq!(receipt.wait().await, TaskCompletion::Canceled);
}
