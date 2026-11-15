//! Command-chain coverage over the existing submission/workspace/storage owner.
//! Only the fixed external SyncTeX operations are substituted.
use super::{Fixture, fixture, request};
use crate::agent::{
    compile_result_commands::{forward_synctex_in, inverse_synctex_in},
    compile_synctex::{
        AgentCompileSynctex, AgentCompileSynctexQuery, SynctexPdfLocation, SynctexSourceLocation,
    },
    repository::execution_workspaces::Binding,
    writing_submission::{WritingSubmissionReceipt, WritingSubmissionService},
    writing_workspace::WritingWorkspaceRequest,
};
use serde_json::{Value, json};
use std::{fs, path::Path};
use uuid::Uuid;

fn adapter() -> AgentCompileSynctex {
    AgentCompileSynctex {
        prepare_toolchain,
        forward: forward_query,
        inverse: inverse_query,
    }
}

fn prepare_toolchain(checked_at: String, runtime_home: &Path) -> Result<String, String> {
    fs::create_dir_all(runtime_home).unwrap();
    fs::write(runtime_home.join("checked_at"), &checked_at).unwrap();
    assert!(checked_at.ends_with('Z'));
    assert_eq!(checked_at.len(), 24);
    Ok("opaque frozen toolchain".into())
}

fn record_query(query: &AgentCompileSynctexQuery<'_>, name: &str, location: Value) {
    assert_eq!(query.toolchain_json, "opaque frozen toolchain");
    assert!(query.runtime_home.join("checked_at").is_file());
    let value = json!({
        "snapshot": query.snapshot,
        "output": query.output_directory,
        "pdf": query.pdf,
        "source": fs::read_to_string(query.snapshot.join("main.tex")).unwrap(),
        "location": location,
    });
    fs::write(query.runtime_home.join(name), value.to_string()).unwrap();
}

fn forward_query(
    query: &AgentCompileSynctexQuery<'_>,
    relative_path: &str,
    line: u32,
    column: u32,
) -> Result<Vec<SynctexPdfLocation>, String> {
    record_query(query, "forward", json!([relative_path, line, column]));
    if query.runtime_home.join("query_error").exists() {
        return Err("original SyncTeX forward cause".into());
    }
    Ok(vec![SynctexPdfLocation {
        page: 2,
        x: 12.5,
        y: 44.0,
        width: 18.0,
        height: 9.0,
    }])
}

fn inverse_query(
    query: &AgentCompileSynctexQuery<'_>,
    page: u32,
    x: f64,
    y: f64,
) -> Result<Vec<SynctexSourceLocation>, String> {
    record_query(query, "inverse", json!([page, x, y]));
    if query.runtime_home.join("query_error").exists() {
        return Err("original SyncTeX inverse cause".into());
    }
    Ok(vec![SynctexSourceLocation {
        relative_path: "main.tex".into(),
        line: 7,
        column: Some(3),
    }])
}

fn result_directory(f: &Fixture, result_id: Uuid) -> std::path::PathBuf {
    f.store
        .writing_cache_directory()
        .join("agent-compile-results")
        .join(result_id.to_string())
}

fn runtime_home(f: &Fixture) -> std::path::PathBuf {
    f.store.writing_cache_directory().join("toolchain-home")
}

fn write_artifacts(f: &Fixture, result_id: Uuid, binding: &Binding) {
    let directory = result_directory(f, result_id);
    fs::create_dir_all(directory.join("source")).unwrap();
    fs::write(directory.join("result.pdf"), b"%PDF-synthetic").unwrap();
    fs::write(directory.join("result.synctex.gz"), b"synthetic SyncTeX").unwrap();
    fs::write(
        directory.join("source/main.tex"),
        b"Retained compile source",
    )
    .unwrap();
    fs::write(
        directory.join("metadata.json"),
        json!({
            "resultId": result_id,
            "identity": {
                "conversationId":binding.conversation_id,
                "requestId":binding.request_id,
                "turnId":binding.turn_id,
                "contextSnapshotId":binding.context_snapshot_id,
                "manuscriptId":binding.manuscript_id,
                "leaseId":binding.lease_id,
                "manifestHash":binding.manifest_hash,
            },
            "createdAtUnixSeconds":std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
            "pdf":true,"synctex":true,"sourceSnapshot":true,
        })
        .to_string(),
    )
    .unwrap();
}

async fn workspace_fixture() -> (Fixture, Binding, Uuid, std::path::PathBuf) {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let prepared = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let workspace = service
        .prepare_workspace(
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: prepared.record.request_id,
                manuscript_id: f.manuscript,
            },
            prepared.record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let working = workspace.lease.working_directory().unwrap();
    let binding = workspace.binding.clone();
    drop(workspace);
    let result_id = Uuid::new_v4();
    write_artifacts(&f, result_id, &binding);
    (f, binding, result_id, working)
}

async fn forward(
    f: &Fixture,
    binding: &Binding,
    result_id: Uuid,
    adapter: impl Into<Option<AgentCompileSynctex>>,
    line: u32,
) -> Result<Vec<SynctexPdfLocation>, String> {
    forward_synctex_in(
        f.store.clone(),
        adapter,
        result_id.to_string(),
        binding.conversation_id.to_string(),
        binding.request_id.to_string(),
        binding.turn_id.to_string(),
        binding.context_snapshot_id.to_string(),
        binding.manuscript_id.to_string(),
        binding.lease_id.to_string(),
        binding.manifest_hash.clone(),
        " main.tex ".into(),
        line,
        3,
    )
    .await
}

async fn inverse(
    f: &Fixture,
    binding: &Binding,
    result_id: Uuid,
    adapter: impl Into<Option<AgentCompileSynctex>>,
    page: u32,
) -> Result<Vec<SynctexSourceLocation>, String> {
    inverse_synctex_in(
        f.store.clone(),
        adapter,
        result_id.to_string(),
        binding.conversation_id.to_string(),
        binding.request_id.to_string(),
        binding.turn_id.to_string(),
        binding.context_snapshot_id.to_string(),
        binding.manuscript_id.to_string(),
        binding.lease_id.to_string(),
        binding.manifest_hash.clone(),
        page,
        12.5,
        44.0,
    )
    .await
}

fn recorded(f: &Fixture, name: &str) -> Value {
    serde_json::from_slice(&fs::read(runtime_home(f).join(name)).unwrap()).unwrap()
}

#[tokio::test]
async fn missing_runtime_selection_preserves_identity_and_location_checks_before_source_failure() {
    let (f, binding, result_id, _) = workspace_fixture().await;
    let mut wrong = binding.clone();
    wrong.manifest_hash = "different".into();
    assert_eq!(
        forward(&f, &wrong, result_id, None, 0).await.unwrap_err(),
        "compile_result_identity_mismatch"
    );
    assert_eq!(
        inverse(&f, &wrong, result_id, None, 0).await.unwrap_err(),
        "compile_result_identity_mismatch"
    );
    assert_eq!(
        forward(&f, &binding, result_id, None, 0).await.unwrap_err(),
        "invalid_path"
    );
    assert_eq!(
        inverse(&f, &binding, result_id, None, 0).await.unwrap_err(),
        "invalid_path"
    );
    assert_eq!(
        forward(&f, &binding, result_id, None, 7).await.unwrap_err(),
        "synctex_unavailable"
    );
    assert_eq!(
        inverse(&f, &binding, result_id, None, 2).await.unwrap_err(),
        "synctex_unavailable"
    );
    assert!(!runtime_home(&f).exists());
}

#[tokio::test]
async fn identity_then_location_checks_reject_before_any_toolchain_operation() {
    let (f, binding, result_id, _) = workspace_fixture().await;
    let mut wrong = binding.clone();
    wrong.manifest_hash = "different".into();
    assert_eq!(
        forward(&f, &wrong, result_id, adapter(), 0)
            .await
            .unwrap_err(),
        "compile_result_identity_mismatch"
    );
    assert_eq!(
        inverse(&f, &wrong, result_id, adapter(), 0)
            .await
            .unwrap_err(),
        "compile_result_identity_mismatch"
    );
    assert_eq!(
        forward(&f, &binding, result_id, adapter(), 0)
            .await
            .unwrap_err(),
        "invalid_path"
    );
    assert_eq!(
        inverse(&f, &binding, result_id, adapter(), 0)
            .await
            .unwrap_err(),
        "invalid_path"
    );
    assert!(!runtime_home(&f).exists());
}

#[tokio::test]
async fn restored_workspace_queries_keep_frozen_paths_and_original_query_errors() {
    let (f, binding, result_id, working) = workspace_fixture().await;
    let pdf = forward(&f, &binding, result_id, adapter(), 7)
        .await
        .unwrap();
    assert_eq!(pdf[0].page, 2);
    let first = recorded(&f, "forward");
    assert_eq!(first["snapshot"], json!(working));
    assert_eq!(first["source"], "Frozen manuscript");
    assert_eq!(first["output"], json!(result_directory(&f, result_id)));
    assert_eq!(
        first["pdf"],
        json!(result_directory(&f, result_id).join("result.pdf"))
    );
    assert_eq!(first["location"], json!(["main.tex", 7, 3]));
    let source = inverse(&f, &binding, result_id, adapter(), 2)
        .await
        .unwrap();
    assert_eq!(source[0].relative_path, "main.tex");
    assert_eq!(recorded(&f, "inverse")["snapshot"], json!(working));
    assert_eq!(recorded(&f, "inverse")["location"], json!([2, 12.5, 44.0]));

    fs::write(
        runtime_home(&f).join("query_error"),
        b"synthetic external failure",
    )
    .unwrap();
    assert_eq!(
        forward(&f, &binding, result_id, adapter(), 7)
            .await
            .unwrap_err(),
        "original SyncTeX forward cause"
    );
    assert_eq!(
        inverse(&f, &binding, result_id, adapter(), 2)
            .await
            .unwrap_err(),
        "original SyncTeX inverse cause"
    );
}

#[tokio::test]
async fn completed_workspace_queries_use_retained_source_without_execution_authority() {
    let (f, binding, result_id, _) = workspace_fixture().await;
    let terminal = research_agent_core::protocol::EventPayload::TurnFinished {
        turn_id: binding.turn_id,
        outcome: research_agent_core::protocol::TurnOutcome::Completed,
    };
    f.store.connection().unwrap().execute(
        "insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)",
        rusqlite::params![f.conversation.to_string(), json!({"payload":terminal}).to_string()],
    ).unwrap();
    forward(&f, &binding, result_id, adapter(), 7)
        .await
        .unwrap();
    inverse(&f, &binding, result_id, adapter(), 2)
        .await
        .unwrap();
    let retained = result_directory(&f, result_id)
        .join("source")
        .canonicalize()
        .unwrap();
    for name in ["forward", "inverse"] {
        let query = recorded(&f, name);
        assert_eq!(query["snapshot"], json!(retained));
        assert_eq!(query["source"], "Retained compile source");
    }
    assert!(
        WritingSubmissionService::from_app_store(&f.store)
            .restore_workspace(
                WritingSubmissionReceipt {
                    conversation_id: binding.conversation_id,
                    request_id: binding.request_id,
                    manuscript_id: binding.manuscript_id,
                },
                binding.context_snapshot_id,
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn restored_binding_mismatch_does_not_fall_back_to_existing_retained_source() {
    let (f, mut binding, result_id, _) = workspace_fixture().await;
    binding.lease_id = Uuid::new_v4();
    write_artifacts(&f, result_id, &binding);
    assert_eq!(
        forward(&f, &binding, result_id, adapter(), 7)
            .await
            .unwrap_err(),
        "compile_result_identity_mismatch"
    );
    assert_eq!(
        inverse(&f, &binding, result_id, adapter(), 2)
            .await
            .unwrap_err(),
        "compile_result_identity_mismatch"
    );
    assert!(!runtime_home(&f).exists());
}

#[tokio::test]
async fn toolchain_preparation_failure_keeps_unavailable_mapping_and_skips_queries() {
    let (f, binding, result_id, _) = workspace_fixture().await;
    let failing = AgentCompileSynctex {
        prepare_toolchain: |_, _| Err("original preparation failure".into()),
        ..adapter()
    };
    assert_eq!(
        forward(&f, &binding, result_id, failing, 7)
            .await
            .unwrap_err(),
        "synctex_unavailable"
    );
    assert_eq!(
        inverse(&f, &binding, result_id, failing, 2)
            .await
            .unwrap_err(),
        "synctex_unavailable"
    );
    assert!(!runtime_home(&f).exists());
}
