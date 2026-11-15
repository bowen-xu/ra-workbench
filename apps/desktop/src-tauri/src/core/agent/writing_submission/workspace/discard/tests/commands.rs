use super::*;
use crate::agent::writing_discard_commands::{DiscardConfirmation, actions};
use crate::agent::writing_workspace_commands::{Error as CommandError, Scope};

fn scope(operation: &journal::Request) -> Scope {
    let b = &operation.binding;
    Scope {
        conversation_id: b.conversation_id,
        request_id: b.request_id,
        manuscript_id: b.manuscript_id,
        context_snapshot_id: b.context_snapshot_id,
    }
}
fn sessions(f: &Fixture) -> Arc<AgentSessionService> {
    Arc::new(AgentSessionService::from_app_store(&f.store).unwrap())
}

#[tokio::test]
async fn discard_commands_preview_without_intent_then_confirm_and_retry() {
    let (f, original) = prepared().await;
    let preview = actions::preview(f.store.clone(), scope(&original))
        .await
        .unwrap();
    let json = serde_json::to_value(preview).unwrap();
    assert_eq!(json["review"]["changes"][0]["path"], "main.tex");
    assert_eq!(
        json["confirmation"]["inventoryHash"],
        original.inventory_hash
    );
    assert_eq!(state(&f, &original), None);
    let request: DiscardConfirmation =
        serde_json::from_value(json["confirmation"].clone()).unwrap();
    let result = actions::execute(f.store.clone(), sessions(&f), request.clone())
        .await
        .unwrap();
    let result = serde_json::to_value(result).unwrap();
    assert_eq!(result["state"], "complete");
    assert_eq!(result["operationId"], json["confirmation"]["operationId"]);
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"Frozen manuscript"
    );
    f.store
        .connection()
        .unwrap()
        .execute("update agent_conversations set archived=1", [])
        .unwrap();
    actions::execute(f.store.clone(), sessions(&f), request)
        .await
        .unwrap();
    assert!(matches!(
        actions::preview(f.store.clone(), scope(&original)).await,
        Err(CommandError::RecoveryRequired)
    ));
}

#[tokio::test]
async fn discard_commands_refuse_stale_inventory_busy_copy_and_unbound_host() {
    let (f, original) = prepared().await;
    let request: DiscardConfirmation = original.clone().into();
    let copies = open(&f.store).unwrap();
    let lease = copies.restore(&original.identity()).unwrap();
    assert!(matches!(
        actions::execute(f.store.clone(), sessions(&f), request.clone()).await,
        Err(CommandError::Busy)
    ));
    std::fs::write(
        lease.working_directory().unwrap().join("main.tex"),
        b"newer",
    )
    .unwrap();
    drop(lease);
    assert!(matches!(
        actions::execute(f.store.clone(), sessions(&f), request.clone()).await,
        Err(CommandError::Conflict)
    ));
    assert_eq!(state(&f, &original), None);
    f.store
        .connection()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(matches!(
        actions::execute(f.store.clone(), sessions(&f), request).await,
        Err(CommandError::Denied)
    ));
    assert!(copies.restore(&original.identity()).is_ok());
}

#[tokio::test]
async fn discard_commands_resume_only_original_intent_after_database_failure() {
    let (f, original) = prepared().await;
    let request: DiscardConfirmation = original.clone().into();
    f.store.connection().unwrap().execute_batch("create trigger fail_release before delete on agent_execution_workspaces begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        actions::execute(f.store.clone(), sessions(&f), request.clone())
            .await
            .is_err()
    );
    assert_eq!(state(&f, &original), Some(journal::State::Pending));
    let mut altered = original.clone();
    altered.operation_id = Uuid::new_v4();
    assert!(matches!(
        actions::execute(f.store.clone(), sessions(&f), altered.into()).await,
        Err(CommandError::Conflict)
    ));
    assert!(matches!(
        actions::preview(f.store.clone(), scope(&original)).await,
        Err(CommandError::RecoveryRequired)
    ));
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_release;")
        .unwrap();
    let complete = actions::execute(f.store.clone(), sessions(&f), request)
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(complete).unwrap()["state"], "complete");
    assert_eq!(state(&f, &original), Some(journal::State::Complete));
}

#[test]
fn discard_command_rejects_extra_execution_fields() {
    let id = Uuid::new_v4();
    let mut value = serde_json::json!({
        "operationId": id, "binding": {
            "scope": {"conversationId":id,"requestId":id,"manuscriptId":id,"contextSnapshotId":id},
            "turnId":id,"leaseId":id,"manifestHash":"a".repeat(64)
        }, "environmentId": "writing", "inventoryHash": "b".repeat(64)
    });
    assert!(serde_json::from_value::<DiscardConfirmation>(value.clone()).is_ok());
    value["force"] = true.into();
    assert!(serde_json::from_value::<DiscardConfirmation>(value).is_err());
}
