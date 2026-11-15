use super::*;
use crate::agent::{
    note_scope::Scope,
    note_steering::{self, RecoveryRequest, SteerRequest},
    repository::steering::SqliteSteeringStore,
    service::SessionTurnInput,
    steering_contract::SteerError,
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{EventPayload, TurnOutcome},
    runner::TurnInputSource,
};

fn scope(f: &Fixture, reading: bool) -> Scope {
    if reading {
        Scope::ReadingNote {
            resource_id: f.page,
        }
    } else {
        Scope::Note {
            resource_id: f.page,
        }
    }
}
async fn saved(f: &Fixture, reading: bool) -> SubmissionRecord {
    let id = Uuid::new_v4();
    if reading {
        reading::prepare(&f.store, &f.models, reading_request(f, id))
            .await
            .unwrap()
    } else {
        prepare(&f.store, &f.models, request(f, id)).await.unwrap()
    }
}
fn command(f: &Fixture, record: &SubmissionRecord, reading: bool, id: Uuid) -> SteerRequest {
    SteerRequest {
        user_input: vec![],
        conversation_id: f.conversation,
        scope: scope(f, reading),
        turn_id: record.turn_id,
        input_id: id,
        text: "Use the retained quote first".into(),
    }
}
fn recovery(f: &Fixture, record: &SubmissionRecord, reading: bool) -> RecoveryRequest {
    RecoveryRequest {
        conversation_id: f.conversation,
        scope: scope(f, reading),
        turn_id: record.turn_id,
    }
}
async fn begin(
    f: &Fixture,
    record: &SubmissionRecord,
) -> std::sync::Arc<crate::agent::service::AgentSessionService> {
    let sessions = super::super::start::sessions(f);
    let id = ConversationId::from_uuid(f.conversation);
    sessions.open(id).await.unwrap();
    sessions
        .commit_payload(
            id,
            record.request_id,
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: record.turn_id,
                context_snapshot_id: record.context_snapshot_id,
                model: record.input.model.clone(),
            },
        )
        .await
        .unwrap();
    sessions
}
#[tokio::test]
async fn text_admission_is_idempotent_frozen_and_same_turn_for_both_note_types() {
    for reading in [false, true] {
        let f = reading_fixture();
        let record = saved(&f, reading).await;
        let input = Uuid::new_v4();
        assert!(matches!(
            note_steering::steer(&f.store, command(&f, &record, reading, input)).await,
            Err(SteerError::Closed)
        ));
        let sessions = begin(&f, &record).await;
        let first = serde_json::to_value(
            note_steering::steer(&f.store, command(&f, &record, reading, input))
                .await
                .unwrap(),
        )
        .unwrap();
        let again = serde_json::to_value(
            note_steering::steer(&f.store, command(&f, &record, reading, input))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first, again);
        assert_eq!(first["turnId"], record.turn_id.to_string());
        assert_eq!(first["inputId"], input.to_string());
        let mut changed = command(&f, &record, reading, input);
        changed.text = "Changed same ID".into();
        assert!(matches!(
            note_steering::steer(&f.store, changed).await,
            Err(SteerError::Conflict)
        ));
        // An explicitly bound second host with the same UUID still cannot relabel the frozen turn.
        assert!(matches!(
            note_steering::steer(&f.store, command(&f, &record, !reading, input)).await,
            Err(SteerError::Closed)
        ));
        let steering = SqliteSteeringStore::new(f.store.shared_agent_connection());
        let source = SessionTurnInput {
            evidence_store: None,
            injected: std::sync::Mutex::default(),
            service: &sessions,
            store: &steering,
            scope: research_agent_core::steering::SteeringScope {
                conversation_id: f.conversation,
                turn_id: record.turn_id,
            },
        };
        let messages = source.drain(record.turn_id, false).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert!(
            source
                .drain(record.turn_id, false)
                .await
                .unwrap()
                .is_empty()
        );
        let original = crate::agent::repository::submissions::read(
            &f.store.connection().unwrap(),
            f.conversation,
            record.request_id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(original, record);
        let db = f.store.connection().unwrap();
        let starts:i64=db.query_row("select count(*) from agent_items where json_extract(payload_json,'$.payload.type')='turnStarted'",[],|r|r.get(0)).unwrap();
        assert_eq!(starts, 1);
    }
}
#[tokio::test]
async fn unconsumed_input_is_recoverable_only_after_terminal_and_never_automatically_requeued() {
    for reading in [false, true] {
        let f = reading_fixture();
        let record = saved(&f, reading).await;
        let sessions = begin(&f, &record).await;
        let consumed = Uuid::new_v4();
        note_steering::steer(&f.store, command(&f, &record, reading, consumed))
            .await
            .unwrap();
        let steering = SqliteSteeringStore::new(f.store.shared_agent_connection());
        SessionTurnInput {
            evidence_store: None,
            injected: std::sync::Mutex::default(),
            service: &sessions,
            store: &steering,
            scope: research_agent_core::steering::SteeringScope {
                conversation_id: f.conversation,
                turn_id: record.turn_id,
            },
        }
        .drain(record.turn_id, false)
        .await
        .unwrap();
        let pending = Uuid::new_v4();
        note_steering::steer(&f.store, command(&f, &record, reading, pending))
            .await
            .unwrap();
        assert!(matches!(
            note_steering::recover(&f.store, recovery(&f, &record, reading)).await,
            Err(SteerError::Closed)
        ));
        sessions
            .commit_payload(
                ConversationId::from_uuid(f.conversation),
                Uuid::new_v4(),
                EventPayload::TurnFinished {
                    turn_id: record.turn_id,
                    outcome: TurnOutcome::Canceled,
                },
            )
            .await
            .unwrap();
        let one = serde_json::to_value(
            note_steering::recover(&f.store, recovery(&f, &record, reading))
                .await
                .unwrap(),
        )
        .unwrap();
        let two = serde_json::to_value(
            note_steering::recover(&f.store, recovery(&f, &record, reading))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(one, two);
        assert_eq!(one["inputs"].as_array().unwrap().len(), 1);
        assert_eq!(one["inputs"][0]["inputId"], pending.to_string());
        assert!(matches!(
            note_steering::recover(&f.store, recovery(&f, &record, !reading)).await,
            Err(SteerError::Closed)
        ));
        // The original accepted receipt remains idempotent after completion; new messages do not.
        note_steering::steer(&f.store, command(&f, &record, reading, pending))
            .await
            .unwrap();
        assert!(matches!(
            note_steering::steer(&f.store, command(&f, &record, reading, Uuid::new_v4())).await,
            Err(SteerError::Closed)
        ));
        assert_eq!(count(&f), 1);
    }
}
#[tokio::test]
async fn retries_recheck_binding_and_original_turn_in_the_admission_transaction() {
    for reading in [false, true] {
        let f = reading_fixture();
        let record = saved(&f, reading).await;
        let _sessions = begin(&f, &record).await;
        let input = Uuid::new_v4();
        note_steering::steer(&f.store, command(&f, &record, reading, input))
            .await
            .unwrap();
        let mut wrong = command(&f, &record, reading, input);
        wrong.turn_id = Uuid::new_v4();
        assert!(matches!(
            note_steering::steer(&f.store, wrong).await,
            Err(SteerError::Closed)
        ));
        f.store.connection().unwrap().execute("delete from agent_conversation_bindings where conversation_id=?1 and resource_kind=?2",params![f.conversation.to_string(),if reading{"readingNote"}else{"note"}]).unwrap();
        assert!(matches!(
            note_steering::steer(&f.store, command(&f, &record, reading, input)).await,
            Err(SteerError::Closed)
        ));
        assert!(matches!(
            note_steering::recover(&f.store, recovery(&f, &record, reading)).await,
            Err(SteerError::Closed)
        ));
        let db = f.store.connection().unwrap();
        let count: i64 = db
            .query_row("select count(*) from agent_steering_inputs", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}
#[tokio::test]
async fn active_gate_sealing_race_never_loses_an_accepted_note_instruction() {
    let f = reading_fixture();
    let record = saved(&f, true).await;
    let sessions = begin(&f, &record).await;
    let steering = SqliteSteeringStore::new(f.store.shared_agent_connection());
    let source = SessionTurnInput {
        evidence_store: None,
        injected: std::sync::Mutex::default(),
        service: &sessions,
        store: &steering,
        scope: research_agent_core::steering::SteeringScope {
            conversation_id: f.conversation,
            turn_id: record.turn_id,
        },
    };
    let (accepted, drained) = tokio::join!(
        note_steering::steer(&f.store, command(&f, &record, true, Uuid::new_v4())),
        source.drain(record.turn_id, true)
    );
    let drained = drained.unwrap();
    match accepted {
        Ok(_) => assert_eq!(drained.len(), 1),
        Err(SteerError::Closed) => assert!(drained.is_empty()),
        Err(other) => panic!("unexpected admission: {other:?}"),
    }
}
#[test]
fn note_steering_wire_cannot_override_model_context_or_authority() {
    let base = serde_json::json!({"conversationId":Uuid::new_v4(),"scope":{"kind":"readingNote","resourceId":Uuid::new_v4()},"turnId":Uuid::new_v4(),"inputId":Uuid::new_v4(),"text":"Question"});
    assert!(serde_json::from_value::<SteerRequest>(base.clone()).is_ok());
    for key in [
        "model",
        "context",
        "tools",
        "contextSnapshotId",
        "allowDisclosure",
    ] {
        let mut value = base.clone();
        value[key] = serde_json::json!(true);
        assert!(serde_json::from_value::<SteerRequest>(value).is_err());
    }
    let mut recovery = base.clone();
    recovery.as_object_mut().unwrap().remove("inputId");
    recovery.as_object_mut().unwrap().remove("text");
    assert!(serde_json::from_value::<RecoveryRequest>(recovery.clone()).is_ok());
    recovery["resubmit"] = serde_json::json!(true);
    assert!(serde_json::from_value::<RecoveryRequest>(recovery).is_err());
}
