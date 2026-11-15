use super::*;
use crate::agent::{
    repository::SqliteConversationStore, service::AgentSessionService,
    writing_workspace_commands as commands,
};
use std::sync::Arc;

#[tokio::test]
async fn consent_commands_use_exact_preview_identity_and_do_not_serialize_source_bytes() {
    let f = fixture(true);
    let record = WritingSubmissionService::from_app_store(&f.store)
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    let scope = commands::Scope {
        conversation_id: f.conversation,
        request_id: record.request_id,
        manuscript_id: f.manuscript,
        context_snapshot_id: record.context_snapshot_id,
    };
    let preview = commands::prepare(
        &f.store,
        commands::PrepareRequest {
            scope: scope.clone(),
            buffers: vec![],
        },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(preview).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["approved"], true);
    assert_eq!(value["policy"], "offline_workspace_v1");
    assert_eq!(value["files"][0]["path"], "main.tex");
    assert!(!value.to_string().contains("Frozen manuscript"));
    assert!(value.get("manifest").is_none());
    let confirmation = commands::Confirmation {
        scope: scope.clone(),
        turn_id: record.turn_id,
        lease_id: Uuid::parse_str(value["leaseId"].as_str().unwrap()).unwrap(),
        manifest_hash: value["manifestHash"].as_str().unwrap().into(),
    };
    let mut wrong = confirmation.clone();
    wrong.manifest_hash = "0".repeat(64);
    assert!(matches!(
        commands::approve(&f.store, wrong).await,
        Err(commands::Error::Conflict)
    ));
    let approved = commands::approve(&f.store, confirmation.clone())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(approved).unwrap()["approved"], true);
    assert_eq!(
        serde_json::to_value(commands::restore(&f.store, scope).await.unwrap()).unwrap()["approved"],
        true
    );
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "commands-test".into())
            .unwrap(),
    )));
    commands::revoke(&f.store, sessions.clone(), confirmation.clone())
        .await
        .unwrap();
    commands::revoke(&f.store, sessions, confirmation.clone())
        .await
        .unwrap();
    assert!(commands::approve(&f.store, confirmation).await.is_err());
}

#[test]
fn ipc_rejects_unknown_execution_scope_and_preserves_editor_identity() {
    let scope = serde_json::json!({"conversationId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"manuscriptId":Uuid::new_v4(),"contextSnapshotId":Uuid::new_v4()});
    assert!(
        serde_json::from_value::<commands::PrepareRequest>(
            serde_json::json!({"scope":scope,"buffers":[],"directory":"/tmp"})
        )
        .is_err()
    );
    let session = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let request:commands::PrepareRequest=serde_json::from_value(serde_json::json!({"scope":scope,"buffers":[{"path":"main.tex","expectedDiskSha256":"0".repeat(64),"sessionId":session,"revision":revision,"text":"buffer"}]})).unwrap();
    assert_eq!(request.buffers[0].session_id, session);
    assert_eq!(request.buffers[0].revision, revision);
    assert!(serde_json::from_value::<commands::Confirmation>(serde_json::json!({"scope":scope,"turnId":Uuid::new_v4(),"leaseId":Uuid::new_v4(),"manifestHash":"0".repeat(64),"network":true})).is_err());
}
