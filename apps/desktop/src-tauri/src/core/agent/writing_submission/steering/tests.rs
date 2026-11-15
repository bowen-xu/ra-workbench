use super::super::tests::{fixture, request};
use super::*;
use crate::agent::{
    repository::SqliteConversationStore,
    service::AgentSessionService,
    writing_steer::{SteerError, SteerRequest, steer},
};
use research_agent_core::{conversation::ConversationId, protocol::EventPayload};
use std::sync::Arc;

#[tokio::test]
async fn command_admits_once_and_rechecks_binding_on_retry() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    let sessions = AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    ));
    let id = ConversationId::from_uuid(record.conversation_id);
    sessions.open(id).await.unwrap();
    let input_id = Uuid::new_v4();
    let command = || SteerRequest {
        user_input: vec![],
        conversation_id: record.conversation_id,
        manuscript_id: f.manuscript,
        turn_id: record.turn_id,
        input_id,
        text: "clarify this".into(),
    };
    assert!(matches!(
        steer(&service, command()).await,
        Err(SteerError::Closed)
    ));
    sessions
        .commit_payload(
            id,
            record.request_id,
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: record.turn_id,
                context_snapshot_id: record.context_snapshot_id,
                model: record.input.model,
            },
        )
        .await
        .unwrap();
    let first = serde_json::to_value(steer(&service, command()).await.unwrap()).unwrap();
    let retry = serde_json::to_value(steer(&service, command()).await.unwrap()).unwrap();
    assert_eq!(first, retry);
    assert_eq!(first["schemaVersion"], 1);
    let changed = SteerRequest {
        text: "different".into(),
        ..command()
    };
    assert!(matches!(
        steer(&service, changed).await,
        Err(SteerError::Conflict)
    ));
    let scope = SteeringScope {
        conversation_id: record.conversation_id,
        turn_id: record.turn_id,
    };
    assert_eq!(
        service.recover_steering(f.manuscript, scope).await,
        Err(SteeringError::Closed)
    );
    sessions
        .commit_payload(
            id,
            record.request_id,
            EventPayload::TurnFinished {
                turn_id: record.turn_id,
                outcome: research_agent_core::protocol::TurnOutcome::Canceled,
            },
        )
        .await
        .unwrap();
    let recovered = service.recover_steering(f.manuscript, scope).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].input_id, input_id);
    assert_eq!(recovered[0].text, "clarify this");
    let response = crate::agent::writing_steering_recovery::read(
        &service,
        crate::agent::writing_steering_recovery::RecoveryRequest {
            conversation_id: scope.conversation_id,
            manuscript_id: f.manuscript,
            turn_id: scope.turn_id,
        },
    )
    .await
    .unwrap();
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["manuscriptId"], f.manuscript.to_string());
    assert_eq!(json["inputs"][0]["inputId"], input_id.to_string());
    assert_eq!(json["inputs"][0]["text"], "clarify this");
    assert_eq!(
        service.recover_steering(Uuid::new_v4(), scope).await,
        Err(SteeringError::Closed)
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [record.conversation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        steer(&service, command()).await,
        Err(SteerError::Closed)
    ));
    assert_eq!(
        service.recover_steering(f.manuscript, scope).await,
        Err(SteeringError::Closed)
    );
    let count: i64 = f
        .store
        .connection()
        .unwrap()
        .query_row("select count(*) from agent_steering_inputs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn command_rejects_model_and_scope_overrides() {
    let base = serde_json::json!({
        "conversationId": Uuid::new_v4(), "manuscriptId": Uuid::new_v4(),
        "turnId": Uuid::new_v4(), "inputId": Uuid::new_v4(), "text": "question",
    });
    assert!(serde_json::from_value::<SteerRequest>(base.clone()).is_ok());
    for field in ["model", "context", "tools", "contextSnapshotId"] {
        let mut request = base.clone();
        request[field] = serde_json::json!("override");
        assert!(serde_json::from_value::<SteerRequest>(request).is_err());
    }
}
