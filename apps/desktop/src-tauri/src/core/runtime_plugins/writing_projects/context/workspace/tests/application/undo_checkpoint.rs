use super::undo::run;
use super::*;
use crate::runtime_plugins::writing_projects::files::{
    self, FileRequest, checkpoints::CheckpointCall,
};
use research_workbench_native_sdk::writing_checkpoint::{CheckpointWrite, DocumentCheckpoint};
use serde_json::json;
use sha2::{Digest, Sha256};

#[tokio::test]
#[ignore = "requires actual independent writing package; undo checkpoint read before source capture"]
async fn actual_package_undo_reads_current_checkpoint_then_completes_mixed_inverse_plan() {
    let (f, binding, scope, session, revision) = super::editor::applied_editor().await;
    let operation = Uuid::new_v4();
    let hash = format!("{:x}", Sha256::digest(b"Revised manuscript"));
    let buffers = json!([{
        "path":"main.tex", "expectedDiskSha256":hash,
        "sessionId":session, "revision":revision, "text":"Revised manuscript",
    }]);
    for action in ["prepare", "start"] {
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":action,"buffers":buffers}),
        )
        .await
        .unwrap();
    }
    run(&f.store, &scope, operation, json!({"action":"defer"}))
        .await
        .unwrap();
    let checkpoint = DocumentCheckpoint {
        schema_version: 1,
        manuscript_id: f.manuscript.to_string(),
        relative_path: "main.tex".into(),
        version: 1,
        content: "Revised manuscript".into(),
        review_baseline: "Frozen manuscript".into(),
        saved_sha256: hash.clone(),
        saved_version: 1,
        operations: vec![],
        save_intent: None,
        editor_receipt: None,
    };
    files::operate_files(
        f.store.clone(),
        binding.service.clone(),
        binding.catalog.clone(),
        binding.package_key.clone(),
        f.manuscript.to_string(),
        FileRequest::Checkpoint {
            call: CheckpointCall::Save {
                write: Box::new(CheckpointWrite {
                    expected_revision: None,
                    checkpoint: checkpoint.clone(),
                }),
            },
        },
    )
    .await
    .unwrap();
    let proof = json!([{"path":"main.tex","revision":1,"stateVersion":1}]);
    let mut stale = proof.clone();
    stale[0]["revision"] = json!(2);
    assert!(
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":"resume","buffers":buffers,"checkpointVersions":stale})
        )
        .await
        .is_err()
    );
    let mut invalid = checkpoint.clone();
    invalid.schema_version = 2;
    let replace = |checkpoint: &DocumentCheckpoint| {
        f.store
            .connection()
            .unwrap()
            .execute(
                "update writing_document_checkpoints set checkpoint_json=?2
             where manuscript_id=?1 and relative_path='main.tex'",
                rusqlite::params![
                    f.manuscript.to_string(),
                    serde_json::to_string(checkpoint).unwrap()
                ],
            )
            .unwrap();
    };
    replace(&invalid);
    let rejected = run(
        &f.store,
        &scope,
        operation,
        json!({
            "action":"resume", "buffers":buffers, "checkpointVersions":proof,
        }),
    )
    .await
    .unwrap_err();
    let rejected = serde_json::to_value(rejected).unwrap();
    assert_eq!(rejected["code"], "invalid_input");
    assert_eq!(rejected["operationId"], operation.to_string());
    assert!(rejected["instanceId"].is_string());
    assert!(rejected["requestId"].is_string());
    assert_eq!(
        run(&f.store, &scope, operation, json!({"action":"read"}))
            .await
            .unwrap()["result"]["status"],
        "recoveryRequired"
    );
    replace(&checkpoint);
    let resumed = run(
        &f.store,
        &scope,
        operation,
        json!({"action":"resume","buffers":buffers,"checkpointVersions":proof}),
    )
    .await
    .unwrap();
    assert_eq!(resumed["result"]["status"], "applying");
    let stored: String = f.store.connection().unwrap().query_row(
        "select checkpoint_json from writing_document_checkpoints where manuscript_id=?1 and relative_path='main.tex'",
        [f.manuscript.to_string()], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        serde_json::from_str::<DocumentCheckpoint>(&stored).unwrap(),
        checkpoint
    );
    let dispatched = run(
        &f.store,
        &scope,
        operation,
        json!({"action":"prepareEditor","current":{
            "path":"main.tex","sessionId":session,"revision":revision,"text":"Revised manuscript",
        }}),
    )
    .await
    .unwrap();
    assert_eq!(dispatched["result"]["text"], "Frozen manuscript");
    run(&f.store, &scope, operation, json!({"action":"confirmEditor","actual":{
        "path":"main.tex","sessionId":session,"revision":Uuid::new_v4(),"text":"Frozen manuscript",
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
                expected_sha256: Some(hash),
            },
        },
    )
    .await
    .unwrap();
    for path in ["new.tex", "remove.tex"] {
        run(
            &f.store,
            &scope,
            operation,
            json!({"action":"applyDisk","path":path}),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        run(&f.store, &scope, operation, json!({"action":"read"}))
            .await
            .unwrap()["result"]["status"],
        "applied"
    );
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert!(!source_root(&f).join("new.tex").exists());
    assert_eq!(
        std::fs::read(source_root(&f).join("remove.tex")).unwrap(),
        b"Retained original"
    );
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}
