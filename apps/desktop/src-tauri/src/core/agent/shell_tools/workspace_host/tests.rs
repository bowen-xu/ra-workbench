use super::*;
use crate::agent::workspace::WorkspaceStore;
use research_agent_core::{model::ToolCall, workspace::WorkspaceManifest};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "tests/hooks.rs"]
mod hooks;
#[path = "tests/stop_hooks.rs"]
mod stop_hooks;

fn scope() -> WorkspaceIdentity {
    WorkspaceIdentity {
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        lease_id: Uuid::new_v4(),
        environment_id: "test".into(),
    }
}
fn call(id: &str, arguments: Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: id.into(),
        name: EXEC.into(),
        arguments,
    }
}
async fn setup(
    root: PathBuf,
    registry: Arc<ProcessRegistry>,
    scope: WorkspaceIdentity,
    allow: Arc<AtomicBool>,
) -> Arc<WorkspaceShellHost> {
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir(root.join("workspaces")).unwrap();
        std::fs::create_dir(root.join("originals")).unwrap();
        std::fs::write(root.join("originals/keep.txt"), "original").unwrap();
        let store = WorkspaceStore::open(&root.join("workspaces"), |_| Ok(())).unwrap();
        let lease = store
            .create(
                WorkspaceManifest {
                    schema_version: 1,
                    identity: scope,
                    files: vec![],
                },
                vec![],
            )
            .unwrap();
        Arc::new(
            WorkspaceShellHost::new(
                lease,
                registry,
                vec![root.join("originals")],
                Arc::new(move |_, _| {
                    if allow.load(Ordering::SeqCst) {
                        Ok(())
                    } else {
                        Err(RunError::ToolRejected)
                    }
                }),
            )
            .unwrap(),
        )
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn real_offline_host_protects_originals_and_controls_and_allows_sequential_work() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let registry = Arc::new(ProcessRegistry::default());
    let scope = scope();
    registry
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let allowed = Arc::new(AtomicBool::new(true));
    let host = setup(
        root.clone(),
        registry.clone(),
        scope.clone(),
        allowed.clone(),
    )
    .await;
    let tools = ShellTools::new(registry.clone(), scope.clone(), host.clone());
    let request = call(
        "first",
        json!({"command":"printf draft > changed.tex; if cat ../manifest.json; then exit 41; fi; if printf corrupt > ../../../originals/keep.txt; then exit 42; fi; printf isolated","yield_ms":5000}),
    );
    let result = tools.authorize_and_execute(request.clone()).await.unwrap();
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["exitCode"], 0, "{}", result.content);
    assert!(!result.is_error);
    assert!(
        value["stdout"]["head"]
            .as_str()
            .unwrap()
            .contains("isolated")
    );
    tools.result_committed(request).await.unwrap();
    assert_eq!(host.changes().await.unwrap().len(), 1);
    let request = call(
        "second",
        json!({"command":"printf second >> changed.tex; test -t 0","terminal":true,"yield_ms":5000}),
    );
    let result = tools.authorize_and_execute(request.clone()).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    tools.result_committed(request).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("originals/keep.txt")).unwrap(),
        "original"
    );
    allowed.store(false, Ordering::SeqCst);
    assert!(
        tools
            .authorize_and_execute(call("denied", json!({"command":"touch denied"})))
            .await
            .is_err()
    );
    assert_eq!(host.changes().await.unwrap().len(), 1);
    registry.shutdown().await.unwrap();
}

#[tokio::test]
async fn closed_turn_and_wrong_registry_cannot_create_execution_intent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let registry = Arc::new(ProcessRegistry::default());
    let scope = scope();
    let host = setup(
        root,
        registry.clone(),
        scope.clone(),
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    let request = ExecRequest {
        command: "touch should-not-exist".into(),
        terminal: false,
        yield_ms: 0,
    };
    assert!(
        host.launch(
            Arc::new(ProcessRegistry::default()),
            scope.clone(),
            request.clone()
        )
        .await
        .is_err()
    );
    assert!(host.launch(registry.clone(), scope, request).await.is_err());
    assert!(host.changes().await.unwrap().is_empty());
    registry.shutdown().await.unwrap();
}

#[tokio::test]
async fn stopped_turn_reaps_sandbox_writer_before_changes_can_be_inspected() {
    let temp = tempfile::tempdir().unwrap();
    let registry = Arc::new(ProcessRegistry::default());
    let scope = scope();
    registry
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let host = setup(
        temp.path().canonicalize().unwrap(),
        registry.clone(),
        scope.clone(),
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    let tools = ShellTools::new(registry.clone(), scope.clone(), host.clone());
    let request = call(
        "running",
        json!({"command":"read value; printf '%s' \"$value\" > changed.tex","yield_ms":0}),
    );
    let result = tools.authorize_and_execute(request.clone()).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&result.content).unwrap()["state"],
        "running"
    );
    tools.result_committed(request).await.unwrap();
    assert!(host.changes().await.is_err());
    registry
        .finish_turn(scope.conversation_id, scope.turn_id)
        .await
        .unwrap();
    assert!(host.changes().await.unwrap().is_empty());
    assert!(
        tools
            .authorize_and_execute(call("late", json!({"command":"touch late"})))
            .await
            .is_err()
    );
    assert!(host.changes().await.unwrap().is_empty());
    registry.shutdown().await.unwrap();
}
