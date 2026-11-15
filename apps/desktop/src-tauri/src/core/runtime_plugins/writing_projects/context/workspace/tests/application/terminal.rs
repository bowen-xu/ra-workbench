use super::*;
use crate::agent::{
    writing_application_commands::{discovery, undo::discovery as undo_discovery},
    writing_delivery_commands as delivery,
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ItemKind, ModelIdentity, TaskKind},
};
use research_workbench_native_sdk::writing_checkpoint::{
    DocumentCheckpoint, DocumentOperation, EditorReceipt,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) async fn applications(
    store: &AppStore,
    manuscript: Uuid,
) -> Result<Value, writing_workspace_commands::Error> {
    discovery::list(
        store,
        discovery::ListRequest {
            manuscript_id: manuscript,
            after: None,
            limit: 64,
        },
    )
    .await
    .map(|page| serde_json::to_value(page).unwrap())
}
pub(super) async fn undos(
    store: &AppStore,
    manuscript: Uuid,
) -> Result<Value, writing_workspace_commands::Error> {
    undo_discovery::list(
        store,
        undo_discovery::ListRequest {
            manuscript_id: manuscript,
            after: None,
            limit: 64,
        },
    )
    .await
    .map(|page| serde_json::to_value(page).unwrap())
}
fn checkpoint(f: &Fixture, scope: &Scope, session: Uuid, revision: Uuid) -> DocumentCheckpoint {
    let proposal = crate::agent::repository::change_sets::read_file_operation_in(
        &f.store.connection().unwrap(),
        scope.conversation_id,
        scope.manuscript_id,
        scope.proposal_id,
    )
    .unwrap();
    let hash = format!("{:x}", Sha256::digest(b"Revised manuscript"));
    DocumentCheckpoint {
        schema_version: 1,
        manuscript_id: f.manuscript.to_string(),
        relative_path: "main.tex".into(),
        version: 1,
        content: "Revised manuscript".into(),
        review_baseline: "Frozen manuscript".into(),
        saved_sha256: hash.clone(),
        saved_version: 0,
        save_intent: None,
        operations: vec![DocumentOperation {
            operation_id: scope.operation_id.to_string(),
            conversation_id: scope.conversation_id.to_string(),
            request_id: proposal.binding.request_id.to_string(),
            result_sha256: hash,
        }],
        editor_receipt: Some(EditorReceipt {
            operation_id: scope.operation_id.to_string(),
            session_id: session.to_string(),
            revision: revision.to_string(),
            text: "Revised manuscript".into(),
            state_version: Some(1),
        }),
    }
}
fn replace_checkpoint(f: &Fixture, checkpoint: &DocumentCheckpoint) {
    f.store.connection().unwrap().execute(
        "update writing_document_checkpoints set checkpoint_json=?2 where manuscript_id=?1 and relative_path='main.tex'",
        rusqlite::params![f.manuscript.to_string(),serde_json::to_string(checkpoint).unwrap()],
    ).unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; original terminal apply discovery"]
async fn actual_package_terminal_apply_discovery_keeps_exact_unsaved_receipts_until_checkpoint_save()
 {
    let (f, binding, scope, session, revision) = super::editor::applied_editor().await;
    let absent = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    assert!(
        applications(&absent, f.manuscript).await.unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut current = checkpoint(&f, &scope, session, revision);
    super::compensation_editor::preserve(&f, &binding, current.clone(), None).await;
    let found = applications(&f.store, f.manuscript).await.unwrap();
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        found["items"][0]["application"]["scope"]["operationId"],
        scope.operation_id.to_string()
    );
    let cause =
        serde_json::to_value(applications(&absent, f.manuscript).await.unwrap_err()).unwrap();
    assert_eq!(cause["code"], "plugin_host_package_unavailable");
    assert_eq!(cause["operationId"], scope.operation_id.to_string());
    let mut invalid = current.clone();
    invalid.schema_version = 2;
    replace_checkpoint(&f, &invalid);
    let cause =
        serde_json::to_value(applications(&f.store, f.manuscript).await.unwrap_err()).unwrap();
    assert_eq!(cause["code"], "invalid_input");
    assert_eq!(cause["operationId"], scope.operation_id.to_string());
    assert!(cause["instanceId"].is_string());
    assert!(cause["requestId"].is_string());
    let mut forged = current.clone();
    forged.editor_receipt.as_mut().unwrap().session_id = Uuid::new_v4().to_string();
    replace_checkpoint(&f, &forged);
    assert!(matches!(
        applications(&f.store, f.manuscript).await,
        Err(writing_workspace_commands::Error::Conflict)
    ));
    replace_checkpoint(&f, &current);
    let paused = discovery::list_with_checkpoints(
        &f.store,
        &discovery::ListRequest {
            manuscript_id: f.manuscript,
            after: None,
            limit: 64,
        },
        |_, _, _| {
            Err(crate::ai::structured_resource_validation::Flow::Need(
                "checkpoint",
            ))
        },
    );
    assert!(matches!(
        paused,
        Err(crate::ai::structured_resource_validation::Flow::Need(
            "checkpoint"
        ))
    ));
    assert!(f.store.connection().unwrap().is_autocommit());
    current.version = 2;
    current.saved_version = 1;
    super::compensation_editor::preserve(&f, &binding, current, Some(1)).await;
    assert!(
        applications(&absent, f.manuscript).await.unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(state(&f.store, &scope).await["status"], "applied");
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; original running proof and provider call transaction"]
async fn actual_package_running_delivery_keeps_saved_checkpoint_and_call_binding_in_original_transaction()
 {
    let (f, binding, scope, session, revision) = super::editor::applied_editor().await;
    let proposal = crate::agent::repository::change_sets::read_file_operation_in(
        &f.store.connection().unwrap(),
        scope.conversation_id,
        scope.manuscript_id,
        scope.proposal_id,
    )
    .unwrap();
    let original = proposal.binding.clone();
    let request = delivery::Acknowledge {
        scope: writing_workspace_commands::Scope {
            conversation_id: original.conversation_id,
            request_id: original.request_id,
            manuscript_id: original.manuscript_id,
            context_snapshot_id: original.context_snapshot_id,
        },
        turn_id: original.turn_id,
    };
    for (ordinal, payload) in [
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: original.turn_id,
            context_snapshot_id: original.context_snapshot_id,
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
        EventPayload::ItemAccepted {
            turn_id: original.turn_id,
            item_id: Uuid::new_v4(),
            kind: ItemKind::ToolCall {
                call_id: Uuid::new_v4(),
                name: "research_deliver_files".into(),
                provider_call_id: Some("terminal-fixture".into()),
                namespace: None,
            },
            text: "{}".into(),
            phase: None,
            source: None,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let event = AgentEvent::new(
            ConversationId::from_uuid(original.conversation_id),
            ordinal as u64 + 1,
            0,
            original.request_id,
            payload,
        )
        .unwrap();
        f.store.connection().unwrap().execute(
            "insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)",
            rusqlite::params![original.conversation_id.to_string(),ordinal,serde_json::to_string(&event).unwrap()],
        ).unwrap();
    }
    let mut current = checkpoint(&f, &scope, session, revision);
    super::compensation_editor::preserve(&f, &binding, current.clone(), None).await;
    let delivery_id = Uuid::new_v4();
    let count = || {
        f.store.connection().unwrap().query_row(
        "select (select count(*) from writing_agent_delivery_proofs)+(select count(*) from writing_agent_delivery_tool_calls)",
        [],|row| row.get::<_,u64>(0),
    ).unwrap()
    };
    assert!(
        delivery::verify_running_with_call(
            &f.store,
            request.clone(),
            delivery_id,
            "terminal-fixture",
            original.clone()
        )
        .await
        .is_err()
    );
    assert_eq!(count(), 0);
    current.version = 2;
    current.saved_version = 1;
    super::compensation_editor::preserve(&f, &binding, current, Some(1)).await;
    let absent = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    let cause = serde_json::to_value(
        delivery::verify_running_with_call(
            &absent,
            request.clone(),
            delivery_id,
            "terminal-fixture",
            original.clone(),
        )
        .await
        .unwrap_err(),
    )
    .unwrap();
    assert_eq!(cause["code"], "plugin_host_package_unavailable");
    assert_eq!(cause["operationId"], scope.operation_id.to_string());
    assert_eq!(count(), 0);
    assert!(
        delivery::verify_running_with_call(
            &f.store,
            request.clone(),
            delivery_id,
            "wrong-call",
            original.clone()
        )
        .await
        .is_err()
    );
    assert_eq!(count(), 0);
    let review = WritingSubmissionService::from_app_store(&f.store)
        .review_workspace_binding(original.clone())
        .await
        .unwrap();
    let paused = delivery::proof_with_checkpoints(
        &f.store,
        &request,
        &review,
        delivery_id,
        Some("terminal-fixture"),
        |_, _, _| {
            Err(crate::ai::structured_resource_validation::Flow::Need(
                "checkpoint",
            ))
        },
    );
    assert!(matches!(
        paused,
        Err(crate::ai::structured_resource_validation::Flow::Need(
            "checkpoint"
        ))
    ));
    assert!(f.store.connection().unwrap().is_autocommit());
    assert_eq!(count(), 0);
    for _ in 0..2 {
        assert_eq!(
            delivery::verify_running_with_call(
                &f.store,
                request.clone(),
                delivery_id,
                "terminal-fixture",
                original.clone()
            )
            .await
            .unwrap(),
            delivery::RunningProof::Applied
        );
        assert_eq!(count(), 2);
    }
    assert!(
        delivery::verify_running_with_call(
            &f.store,
            request,
            Uuid::new_v4(),
            "terminal-fixture",
            original
        )
        .await
        .is_err()
    );
    assert_eq!(count(), 2);
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
