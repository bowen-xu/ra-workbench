use super::*;
use crate::agent::writing_application_commands::undo_restoration;
use crate::runtime_plugins::writing_projects::files::{self, FileRequest};
use research_workbench_native_sdk::writing_checkpoint::{
    DocumentCheckpoint, DocumentOperation, EditorReceipt,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

async fn restore(
    store: &AppStore,
    scope: &Scope,
    undo: Uuid,
    mut request: Value,
) -> Result<Value, writing_workspace_commands::Error> {
    let mut current = serde_json::to_value(scope).unwrap();
    current["operationId"] = json!(undo);
    request["scope"] = current;
    undo_restoration::execute(
        store,
        serde_json::from_value(json!({"originalScope":scope,"request":request})).unwrap(),
    )
    .await
    .map(|reply| serde_json::to_value(reply).unwrap())
}

#[tokio::test]
#[ignore = "requires actual independent writing package; rebuilt inverse editor compensation"]
async fn actual_package_undo_compensation_restores_rebuilt_editor_without_rebinding_receipts() {
    let (f, binding, scope, session, revision) = super::editor::applied_editor().await;
    let operation = Uuid::new_v4();
    let revised_hash = format!("{:x}", Sha256::digest(b"Revised manuscript"));
    let frozen_hash = format!("{:x}", Sha256::digest(b"Frozen manuscript"));
    let buffers = json!([{"path":"main.tex","sessionId":session,"revision":revision,
        "text":"Revised manuscript","expectedDiskSha256":revised_hash}]);
    for action in ["prepare", "start"] {
        super::undo::run(
            &f.store,
            &scope,
            operation,
            json!({"action":action,"buffers":buffers}),
        )
        .await
        .unwrap();
    }
    super::undo::run(
        &f.store,
        &scope,
        operation,
        json!({"action":"prepareEditor","current":{
            "path":"main.tex","sessionId":session,"revision":revision,"text":"Revised manuscript",
        }}),
    )
    .await
    .unwrap();
    let applied_revision = Uuid::new_v4();
    super::undo::run(&f.store, &scope, operation, json!({"action":"confirmEditor","actual":{
        "path":"main.tex","sessionId":session,"revision":applied_revision,"text":"Frozen manuscript",
    }})).await.unwrap();
    files::operate_files(
        f.store.clone(),
        binding.service.clone(),
        binding.catalog.clone(),
        binding.package_key.clone(),
        f.manuscript.to_string(),
        FileRequest::Save {
            request: research_workbench_native_sdk::writing_project::SaveFileRequest {
                manuscript_id: f.manuscript.to_string(),
                relative_path: "main.tex".into(),
                content: "Frozen manuscript".into(),
                expected_sha256: Some(revised_hash.clone()),
            },
        },
    )
    .await
    .unwrap();
    super::undo::run(&f.store, &scope, operation, json!({"action":"defer"}))
        .await
        .unwrap();
    let checked = application_scope(&scope);
    let proposal = crate::agent::repository::change_sets::read_file_operation_in(
        &f.store.connection().unwrap(),
        checked.conversation,
        checked.manuscript,
        checked.proposal,
    )
    .unwrap();
    let mut checkpoint = DocumentCheckpoint {
        schema_version: 1,
        manuscript_id: f.manuscript.to_string(),
        relative_path: "main.tex".into(),
        version: 1,
        content: "Frozen manuscript".into(),
        review_baseline: "Revised manuscript".into(),
        saved_sha256: frozen_hash.clone(),
        saved_version: 1,
        save_intent: None,
        operations: vec![
            DocumentOperation {
                operation_id: scope.operation_id.to_string(),
                conversation_id: scope.conversation_id.to_string(),
                request_id: proposal.binding.request_id.to_string(),
                result_sha256: revised_hash.clone(),
            },
            DocumentOperation {
                operation_id: operation.to_string(),
                conversation_id: scope.conversation_id.to_string(),
                request_id: proposal.binding.request_id.to_string(),
                result_sha256: frozen_hash.clone(),
            },
        ],
        editor_receipt: Some(EditorReceipt {
            operation_id: operation.to_string(),
            session_id: session.to_string(),
            revision: applied_revision.to_string(),
            text: "Frozen manuscript".into(),
            state_version: Some(1),
        }),
    };
    super::compensation_editor::preserve(&f, &binding, checkpoint.clone(), None).await;
    let rebuilt_session = Uuid::new_v4();
    let rebuilt_revision = Uuid::new_v4();
    let restore_id = Uuid::new_v4();
    let current = json!({"path":"main.tex","sessionId":rebuilt_session,"revision":rebuilt_revision,
        "text":"Frozen manuscript"});
    let prepare = |proof| {
        json!({"action":"prepare","restoreId":restore_id,"current":current,
        "checkpointVersion":proof})
    };
    let proof = json!({"path":"main.tex","revision":1,"stateVersion":1});
    let absent = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    for stale in [
        Value::Null,
        json!({"path":"main.tex","revision":2,"stateVersion":1}),
    ] {
        assert!(
            restore(&f.store, &scope, operation, prepare(stale))
                .await
                .is_err()
        );
    }
    let cause = serde_json::to_value(
        restore(&absent, &scope, operation, prepare(proof.clone()))
            .await
            .unwrap_err(),
    )
    .unwrap();
    assert_eq!(cause["code"], "plugin_host_package_unavailable");
    assert_eq!(cause["operationId"], operation.to_string());
    let files_before = super::undo::run(&f.store, &scope, operation, json!({"action":"read"}))
        .await
        .unwrap()["result"]["files"]
        .clone();
    let edit = restore(&f.store, &scope, operation, prepare(proof))
        .await
        .unwrap();
    assert_eq!(edit["result"]["kind"], "edit");
    assert_eq!(edit["result"]["text"], "Revised manuscript");
    assert_eq!(
        edit["result"]["expected"]["sessionId"],
        rebuilt_session.to_string()
    );
    assert_eq!(
        edit["result"]["expected"]["revision"],
        rebuilt_revision.to_string()
    );
    let replay = restore(&absent,&scope,operation,json!({"action":"prepare","restoreId":restore_id,
        "current":{"path":"main.tex","sessionId":Uuid::new_v4(),"revision":Uuid::new_v4(),"text":"later input"}}))
        .await.unwrap();
    assert_eq!(replay["result"]["kind"], "state");
    let restored_revision = Uuid::new_v4();
    restore(&f.store,&scope,operation,json!({"action":"confirm","restoreId":restore_id,"actual":{
        "path":"main.tex","sessionId":rebuilt_session,"revision":restored_revision,"text":"Revised manuscript",
    }})).await.unwrap();
    let saved = super::undo::run(&f.store, &scope, operation, json!({"action":"read"}))
        .await
        .unwrap();
    assert_eq!(
        saved["result"]["files"][0]["target"],
        files_before[0]["target"]
    );
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    checkpoint.version = 2;
    checkpoint.content = "Revised manuscript".into();
    checkpoint.operations.push(DocumentOperation {
        operation_id: restore_id.to_string(),
        conversation_id: scope.conversation_id.to_string(),
        request_id: proposal.binding.request_id.to_string(),
        result_sha256: revised_hash.clone(),
    });
    checkpoint.editor_receipt = Some(EditorReceipt {
        operation_id: restore_id.to_string(),
        session_id: rebuilt_session.to_string(),
        revision: restored_revision.to_string(),
        text: "Revised manuscript".into(),
        state_version: Some(2),
    });
    super::compensation_editor::preserve(&f, &binding, checkpoint.clone(), Some(1)).await;
    let done = super::undo::run(
        &f.store,
        &scope,
        operation,
        json!({"action":"rollbackDisk",
        "buffers":[{"path":"main.tex","sessionId":rebuilt_session,"revision":restored_revision,
            "text":"Revised manuscript","expectedDiskSha256":frozen_hash}],
        "checkpointVersions":[{"path":"main.tex","revision":2,"stateVersion":2}]}),
    )
    .await
    .unwrap();
    assert_eq!(done["result"]["status"], "rolledBack");
    let pending = super::terminal::undos(&f.store, f.manuscript)
        .await
        .unwrap();
    assert_eq!(pending["items"].as_array().unwrap().len(), 1);
    let cause = serde_json::to_value(
        super::terminal::undos(&absent, f.manuscript)
            .await
            .unwrap_err(),
    )
    .unwrap();
    assert_eq!(cause["code"], "plugin_host_package_unavailable");
    assert_eq!(cause["operationId"], operation.to_string());
    files::operate_files(
        f.store.clone(),
        binding.service.clone(),
        binding.catalog.clone(),
        binding.package_key.clone(),
        f.manuscript.to_string(),
        FileRequest::Save {
            request: research_workbench_native_sdk::writing_project::SaveFileRequest {
                manuscript_id: f.manuscript.to_string(),
                relative_path: "main.tex".into(),
                content: "Revised manuscript".into(),
                expected_sha256: Some(frozen_hash),
            },
        },
    )
    .await
    .unwrap();
    checkpoint.version = 3;
    checkpoint.saved_version = 2;
    checkpoint.saved_sha256 = revised_hash;
    super::compensation_editor::preserve(&f, &binding, checkpoint, Some(2)).await;
    assert!(
        super::terminal::undos(&absent, f.manuscript).await.unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    assert!(!source_root(&f).join("remove.tex").exists());
    assert_eq!(
        std::fs::read(source_root(&f).join("new.tex")).unwrap(),
        b"Added manuscript"
    );
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
