use super::*;
use crate::agent::{
    repository::proposal_applications::{self as applications, ApplicationScope},
    writing_application_commands::{self as commands, Request, Scope},
};

async fn proposal() -> (Fixture, Binding, Scope) {
    let (f, record, binding) = input().await;
    std::fs::create_dir_all(f.store.content_directory()).unwrap();
    crate::runtime_plugins::writing_projects::files::operate_files(
        f.store.clone(),
        binding.service.clone(),
        binding.catalog.clone(),
        binding.package_key.clone(),
        f.manuscript.to_string(),
        crate::runtime_plugins::writing_projects::files::FileRequest::Create {
            request: research_workbench_native_sdk::writing_project::SaveFileRequest {
                manuscript_id: f.manuscript.to_string(),
                relative_path: "remove.tex".into(),
                content: "Retained original".into(),
                expected_sha256: None,
            },
        },
    )
    .await
    .unwrap();
    let service = WritingSubmissionService::from_app_store(&f.store);
    let receipt = WritingSubmissionReceipt {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        manuscript_id: f.manuscript,
    };
    let copy = service
        .prepare_workspace(
            receipt,
            record.context_snapshot_id,
            WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let root = copy.lease.working_directory().unwrap();
    std::fs::write(root.join("main.tex"), b"Revised manuscript").unwrap();
    std::fs::write(root.join("new.tex"), b"Added manuscript").unwrap();
    std::fs::remove_file(root.join("remove.tex")).unwrap();
    drop(copy);
    let reviewed = service
        .review_workspace(receipt, record.context_snapshot_id)
        .await
        .unwrap();
    assert_eq!(reviewed.changes.len(), 3);
    let proposal_id = service
        .publish_workspace_changes(reviewed.binding, reviewed.changes)
        .await
        .unwrap();
    let scope = Scope {
        conversation_id: record.conversation_id,
        manuscript_id: f.manuscript,
        proposal_id,
        operation_id: Uuid::new_v4(),
    };
    (f, binding, scope)
}
async fn state(store: &AppStore, scope: &Scope) -> serde_json::Value {
    serde_json::to_value(
        commands::execute(
            store,
            Request::Read {
                scope: scope.clone(),
            },
        )
        .await
        .unwrap(),
    )
    .unwrap()
}
fn application_scope(scope: &Scope) -> ApplicationScope {
    ApplicationScope {
        conversation: scope.conversation_id,
        manuscript: scope.manuscript_id,
        proposal: scope.proposal_id,
        operation: scope.operation_id,
    }
}
fn recovery(store: &AppStore, scope: &Scope) {
    let mut db = store.connection().unwrap();
    let mut tx = db.transaction().unwrap();
    applications::require_recovery_in(&mut tx, application_scope(scope)).unwrap();
    tx.commit().unwrap();
}
fn source_error(
    result: Result<commands::Response, writing_workspace_commands::Error>,
    scope: &Scope,
) {
    let error = serde_json::to_value(result.err().expect("missing source")).unwrap();
    assert_eq!(error["code"], "plugin_host_package_unavailable");
    assert_eq!(error["conversationId"], scope.conversation_id.to_string());
    assert_eq!(error["manuscriptId"], scope.manuscript_id.to_string());
    assert_eq!(error["operationId"], scope.operation_id.to_string());
}

#[tokio::test]
#[ignore = "requires actual independent writing package; C04/C05 production application admission"]
async fn actual_package_application_begin_resume_keep_preflight_replay_and_failure_identity() {
    let (f, binding, scope) = proposal().await;
    let missing = f.store.clone().with_native_writing_context(Binding {
        package_key: "0".repeat(64),
        ..binding.clone()
    });
    let mut foreign = scope.clone();
    foreign.conversation_id = Uuid::new_v4();
    let refused = commands::execute(
        &missing,
        Request::Begin {
            scope: foreign,
            buffers: vec![],
        },
    )
    .await;
    assert!(
        serde_json::to_value(refused.err().expect("foreign binding"))
            .unwrap()
            .is_string()
    );
    source_error(
        commands::execute(
            &missing,
            Request::Begin {
                scope: scope.clone(),
                buffers: vec![],
            },
        )
        .await,
        &scope,
    );
    assert!(
        !applications::exists_in(
            &f.store.connection().unwrap(),
            scope.conversation_id,
            scope.manuscript_id,
            scope.proposal_id,
            scope.operation_id
        )
        .unwrap()
    );
    let begun = serde_json::to_value(
        commands::execute(
            &f.store,
            Request::Begin {
                scope: scope.clone(),
                buffers: vec![],
            },
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(begun["status"], "applying");
    assert_eq!(begun["files"].as_array().unwrap().len(), 3);
    let repeated = serde_json::to_value(
        commands::execute(
            &missing,
            Request::Begin {
                scope: scope.clone(),
                buffers: vec![],
            },
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(begun, repeated);
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source");
    assert_eq!(
        std::fs::read(source.join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    assert!(!source.join("new.tex").exists());
    recovery(&f.store, &scope);
    let paused = state(&f.store, &scope).await;
    assert_eq!(paused["status"], "recoveryRequired");
    source_error(
        commands::execute(
            &missing,
            Request::Resume {
                scope: scope.clone(),
                buffers: vec![],
            },
        )
        .await,
        &scope,
    );
    assert_eq!(state(&f.store, &scope).await, paused);
    std::fs::write(source.join("main.tex"), b"Concurrent manuscript").unwrap();
    assert!(
        commands::execute(
            &f.store,
            Request::Resume {
                scope: scope.clone(),
                buffers: vec![]
            }
        )
        .await
        .is_err()
    );
    assert_eq!(state(&f.store, &scope).await, paused);
    std::fs::write(source.join("main.tex"), b"Frozen manuscript").unwrap();
    let resumed = serde_json::to_value(
        commands::execute(
            &f.store,
            Request::Resume {
                scope: scope.clone(),
                buffers: vec![],
            },
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(resumed, begun);
    {
        let mut db = f.store.connection().unwrap();
        let mut tx = db.transaction().unwrap();
        applications::dispatch_in(&tx, application_scope(&scope), "main.tex").unwrap();
        applications::require_recovery_in(&mut tx, application_scope(&scope)).unwrap();
        tx.commit().unwrap();
    }
    let refused = commands::execute(
        &missing,
        Request::Resume {
            scope: scope.clone(),
            buffers: vec![],
        },
    )
    .await;
    assert_eq!(
        serde_json::to_value(refused.err().expect("dispatched phase")).unwrap(),
        "conflict"
    );
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

async fn begin(store: &AppStore, scope: &Scope) {
    commands::execute(
        store,
        Request::Begin {
            scope: scope.clone(),
            buffers: vec![],
        },
    )
    .await
    .unwrap();
}
async fn apply(
    store: &AppStore,
    scope: &Scope,
    path: &str,
) -> Result<commands::Response, writing_workspace_commands::Error> {
    commands::execute(
        store,
        Request::ApplyDisk {
            scope: scope.clone(),
            path: path.into(),
        },
    )
    .await
}
fn source_root(f: &Fixture) -> std::path::PathBuf {
    f.store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source")
}

#[path = "application/disk.rs"]
mod disk;

#[path = "application/editor.rs"]
mod editor;

#[path = "application/undo.rs"]
mod undo;

#[path = "application/undo_checkpoint.rs"]
mod undo_checkpoint;

#[path = "application/compensation.rs"]
mod compensation;

#[path = "application/compensation_editor.rs"]
mod compensation_editor;

#[path = "application/compensation_pause.rs"]
mod compensation_pause;

#[path = "application/undo_editor_restoration.rs"]
mod undo_editor_restoration;

#[path = "application/editor_restoration_pause.rs"]
mod editor_restoration_pause;

#[path = "application/terminal.rs"]
mod terminal;
