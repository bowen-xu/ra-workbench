use super::*;
use crate::agent::{
    process::{
        ProcessLimits, ProcessPoll, ProcessRegistry, ProcessRegistryError, ProcessSession,
        ProcessSpec,
    },
    shell_tools::{EXEC, ExecRequest, ShellHost, ShellRequest, ShellTools},
};
use research_agent_core::workspace::WorkspaceIdentity;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio::sync::Notify;

struct Host {
    root: PathBuf,
    launched: Notify,
    process: Mutex<Option<Uuid>>,
}

impl ShellHost for Host {
    fn authorize(&self, _: WorkspaceIdentity, _: ShellRequest) -> RunFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn launch(
        &self,
        registry: Arc<ProcessRegistry>,
        scope: WorkspaceIdentity,
        _: ExecRequest,
    ) -> RunFuture<'_, Uuid> {
        let root = self.root.clone();
        Box::pin(async move {
            // Synthetic subprocess only. Production authorization and sandbox
            // preparation remain in WorkspaceShellHost.
            let process = tokio::task::spawn_blocking(move || {
                registry.start(scope, || {
                    ProcessSession::spawn(
                        ProcessSpec {
                            program: "/bin/cat".into(),
                            arguments: vec![],
                            directory: root,
                            environment: BTreeMap::new(),
                        },
                        ProcessLimits {
                            retained_bytes: 128,
                            execution_timeout: Duration::from_secs(10),
                            drain_timeout: Duration::from_secs(2),
                        },
                    )
                })
            })
            .await
            .map_err(|_| RunError::EventSink)?
            .map_err(|_| RunError::EventSink)?;
            *self.process.lock().unwrap() = Some(process);
            self.launched.notify_one();
            Ok(process)
        })
    }
}

#[tokio::test]
async fn canceled_wrapper_joins_shell_owner_and_retains_result_until_acknowledged() {
    let temp = tempfile::tempdir().unwrap();
    let registry = Arc::new(ProcessRegistry::default());
    let scope = WorkspaceIdentity {
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        lease_id: Uuid::new_v4(),
        environment_id: "test".into(),
    };
    registry
        .begin_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    let host = Arc::new(Host {
        root: temp.path().into(),
        launched: Notify::new(),
        process: Mutex::new(None),
    });
    let tools = ShellTools::new(registry.clone(), scope.clone(), host.clone());
    let cancellation = AgentCancellation::default();
    let wrapper = CancellableTools {
        inner: &tools,
        cancellation: cancellation.clone(),
    };
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "shell".into(),
        name: EXEC.into(),
        arguments: serde_json::json!({"command":"synthetic stdin wait", "yield_ms":5000}),
    };
    assert!(!wrapper.supports_parallel(&call));
    assert!(!wrapper.cancellation_safe_to_drop(&call));
    let mut execution = wrapper.authorize_and_execute(call.clone());
    tokio::select! {
        _ = host.launched.notified() => {},
        _ = &mut execution => panic!("cat must be waiting for input"),
    }
    // Match service/run.rs: close admission and signal the process before
    // awaiting the still-owned runner/tool operation.
    registry
        .stop_turn(scope.conversation_id, scope.turn_id)
        .unwrap();
    cancellation.cancel();
    let output = tokio::time::timeout(Duration::from_secs(4), execution)
        .await
        .unwrap()
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(value["state"], "cancelled");
    let process = host.process.lock().unwrap().unwrap();
    assert!(matches!(
        registry
            .poll(&scope, process, Duration::ZERO)
            .await
            .unwrap(),
        ProcessPoll::Finished(_)
    ));
    wrapper.result_committed(call).await.unwrap();
    assert!(matches!(
        registry.poll(&scope, process, Duration::ZERO).await,
        Err(ProcessRegistryError::NotFound)
    ));
    registry
        .finish_turn(scope.conversation_id, scope.turn_id)
        .await
        .unwrap();
}
