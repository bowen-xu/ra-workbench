use super::*;
use crate::agent::{writing_review_commands as commands, writing_workspace_commands::Scope};

#[tokio::test]
async fn review_commands_bind_file_bytes_to_exact_inventory_without_execution_access() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    let scope = Scope {
        conversation_id: f.conversation,
        request_id: record.request_id,
        manuscript_id: f.manuscript,
        context_snapshot_id: record.context_snapshot_id,
    };
    let copy = service
        .prepare_workspace(
            scope.receipt(),
            scope.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let path = copy.lease.working_directory().unwrap().join("main.tex");
    std::fs::write(&path, b"Edited source").unwrap();
    drop(copy);
    let inventory =
        serde_json::to_value(commands::review(&f.store, scope.clone()).await.unwrap()).unwrap();
    assert_eq!(inventory["schemaVersion"], 1);
    assert_eq!(inventory["changes"][0]["kind"], "modified");
    assert_eq!(inventory["changes"][0]["before"]["byteSize"], 17);
    assert!(!inventory.to_string().contains("Frozen manuscript"));
    assert!(!inventory.to_string().contains("Edited source"));
    let request = serde_json::json!({ "binding": inventory["binding"], "path": "main.tex",
        "before": inventory["changes"][0]["before"], "after": inventory["changes"][0]["after"] });
    let body = serde_json::to_value(
        commands::review_file(&f.store, serde_json::from_value(request.clone()).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        body["before"],
        serde_json::json!({"kind":"text","text":"Frozen manuscript"})
    );
    assert_eq!(
        body["after"],
        serde_json::json!({"kind":"text","text":"Edited source"})
    );
    assert_eq!(body["binding"], inventory["binding"]);
    for field in ["turnId", "leaseId", "manifestHash"] {
        let mut wrong = request.clone();
        wrong["binding"][field] = if field == "manifestHash" {
            serde_json::json!("0".repeat(64))
        } else {
            serde_json::json!(Uuid::new_v4())
        };
        assert!(
            commands::review_file(&f.store, serde_json::from_value(wrong).unwrap())
                .await
                .is_err()
        );
    }
    let mut wrong = request.clone();
    wrong["path"] = serde_json::json!("../main.tex");
    assert!(matches!(
        commands::review_file(&f.store, serde_json::from_value(wrong).unwrap()).await,
        Err(crate::agent::writing_workspace_commands::Error::InvalidInput)
    ));
    wrong = request.clone();
    wrong["arbitraryRoot"] = serde_json::json!("/tmp");
    assert!(serde_json::from_value::<commands::FileRequest>(wrong).is_err());
    std::fs::write(&path, b"Changed after inventory").unwrap();
    assert!(
        commands::review_file(&f.store, serde_json::from_value(request).unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn review_history_returns_only_scoped_identities_and_validates_inputs() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    let scope = Scope {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        request_id: record.request_id,
        context_snapshot_id: record.context_snapshot_id,
    };
    let copy = service
        .prepare_workspace(
            scope.receipt(),
            scope.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    // Discovery must also work while the execution lease is locked.
    let input = serde_json::json!({"conversationId":f.conversation,"manuscriptId":f.manuscript,"cursor":null,"limit":20});
    let page = serde_json::to_value(
        commands::history(&f.store, serde_json::from_value(input.clone()).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page["schemaVersion"], 1);
    assert_eq!(page["nextCursor"], serde_json::Value::Null);
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        page["entries"][0]["scope"],
        serde_json::to_value(&scope).unwrap()
    );
    assert_eq!(page["entries"][0]["turnId"], record.turn_id.to_string());
    assert!(!page.to_string().contains("Frozen manuscript"));
    assert!(!page.to_string().contains("leaseId"));
    drop(copy);
    for (field, value) in [
        ("limit", serde_json::json!(0)),
        ("cursor", serde_json::json!(Uuid::nil())),
        ("manuscriptId", serde_json::json!(Uuid::nil())),
    ] {
        let mut invalid = input.clone();
        invalid[field] = value;
        assert!(matches!(
            commands::history(&f.store, serde_json::from_value(invalid).unwrap()).await,
            Err(crate::agent::writing_workspace_commands::Error::InvalidInput)
        ));
    }
    let mut wrong = input;
    wrong["manuscriptId"] = serde_json::json!(Uuid::new_v4());
    assert!(
        commands::history(&f.store, serde_json::from_value(wrong).unwrap())
            .await
            .is_err()
    );
}
