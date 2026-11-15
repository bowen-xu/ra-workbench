use super::*;
use crate::agent::{
    process::ProcessRegistry,
    shell_tools::{EXEC, ExecRequest, ShellHost, ShellTools, workspace_host::WorkspaceShellHost},
    workspace::{WorkspaceInput, WorkspaceStore},
};
use research_agent_core::{model::ToolCall, runner::ToolExecutor};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn durable_consent_controls_real_host_and_revocation_reaches_existing_host() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let registry = Arc::new(ProcessRegistry::default());
    let native_registry = registry.clone();
    let (connection, binding, scope, host) = tokio::task::spawn_blocking(move || {
        std::fs::create_dir(root.join("copies")).unwrap();
        std::fs::create_dir(root.join("protected")).unwrap();
        let mut db = Connection::open(root.join("protected/db.sqlite")).unwrap();
        let (request, manuscript, manifest) = setup(&mut db);
        let scope = manifest.identity.clone();
        let store = WorkspaceStore::open(&root.join("copies"), |_| Ok(())).unwrap();
        let lease = store
            .create(
                manifest.clone(),
                vec![WorkspaceInput {
                    path: "main.tex".into(),
                    bytes: b"source".to_vec(),
                }],
            )
            .unwrap();
        let tx = db.transaction().unwrap();
        let binding = prepare_in(&tx, request, manuscript, &manifest).unwrap();
        tx.commit().unwrap();
        let connection = Arc::new(Mutex::new(db));
        // Source disclosure/preparation alone cannot construct an executing host.
        assert!(
            WorkspaceShellHost::from_approved_binding(
                lease,
                native_registry.clone(),
                vec![root.join("protected")],
                connection.clone(),
                binding.clone(),
            )
            .is_err()
        );
        {
            let mut db = connection.lock().unwrap();
            let tx = db.transaction().unwrap();
            approve_in(&tx, &binding).unwrap();
            tx.commit().unwrap();
        }
        let host = WorkspaceShellHost::from_approved_binding(
            store.restore(&scope).unwrap(),
            native_registry,
            vec![root.join("protected")],
            connection.clone(),
            binding.clone(),
        )
        .unwrap();
        (connection, binding, scope, Arc::new(host))
    })
    .await
    .unwrap();
    registry
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let tools = ShellTools::new(registry.clone(), scope.clone(), host.clone());
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "approved".into(),
        name: EXEC.into(),
        arguments: serde_json::json!({"command":"printf changed > main.tex", "yield_ms":5000}),
    };
    let result = tools.authorize_and_execute(call.clone()).await.unwrap();
    assert!(!result.is_error, "{}", result.content);
    tools.result_committed(call).await.unwrap();
    assert_eq!(host.changes().await.unwrap().len(), 1);
    tokio::task::spawn_blocking(move || {
        let mut db = connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        revoke_in(&tx, binding.conversation_id, binding.request_id).unwrap();
        tx.commit().unwrap();
    })
    .await
    .unwrap();
    assert!(
        tools
            .authorize_and_execute(ToolCall {
                namespace: None,
                raw_arguments_json: None,
                call_id: "revoked".into(),
                name: EXEC.into(),
                arguments: serde_json::json!({"command":"touch forbidden"}),
            })
            .await
            .is_err()
    );
    // Launch itself rechecks: bypassing the tool's earlier check cannot reuse consent.
    assert!(
        host.launch(
            registry.clone(),
            scope,
            ExecRequest {
                command: "touch forbidden".into(),
                terminal: false,
                yield_ms: 0,
            }
        )
        .await
        .is_err()
    );
    assert_eq!(host.changes().await.unwrap().len(), 1);
    registry.shutdown().await.unwrap();
}
