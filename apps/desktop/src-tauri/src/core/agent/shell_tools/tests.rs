use super::*;
use crate::agent::process::{ProcessLimits, ProcessRegistryError, ProcessSession, ProcessSpec};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Host {
    root: PathBuf,
    allowed: AtomicBool,
    launches: AtomicUsize,
    selected_scope: Mutex<Option<WorkspaceIdentity>>,
}

#[test]
fn shell_schema_errors_are_recoverable_without_a_process_owner() {
    for args in [
        json!([]),
        json!(null),
        json!(17),
        json!({}),
        json!({"command":17}),
    ] {
        assert!(matches!(
            ShellTools::parse(&call("invalid", EXEC, args)),
            Err(RunError::ToolArguments(_))
        ));
    }
    assert!(matches!(
        ShellTools::parse(&call("invalid", "unknown", json!({}))),
        Err(RunError::ToolRejected)
    ));
}
impl ShellHost for Host {
    fn scope(&self, original: WorkspaceIdentity) -> RunFuture<'_, WorkspaceIdentity> {
        let selected = self.selected_scope.lock().unwrap().clone();
        Box::pin(async move { Ok(selected.unwrap_or(original)) })
    }
    fn authorize(&self, _: WorkspaceIdentity, _: ShellRequest) -> RunFuture<'_, ()> {
        Box::pin(async {
            if self.allowed.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(RunError::ToolRejected)
            }
        })
    }
    fn launch(
        &self,
        registry: Arc<ProcessRegistry>,
        scope: WorkspaceIdentity,
        args: ExecRequest,
    ) -> RunFuture<'_, Uuid> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        let root = self.root.clone();
        Box::pin(async move {
            // Test-only launcher. Production host must apply workspace/sandbox policy.
            tokio::task::spawn_blocking(move || {
                registry.start(scope, || {
                    let spec = ProcessSpec {
                        program: "/bin/sh".into(),
                        arguments: vec!["-c".into(), args.command.into()],
                        directory: root,
                        environment: BTreeMap::new(),
                    };
                    let limits = ProcessLimits {
                        retained_bytes: 1024,
                        execution_timeout: Duration::from_secs(10),
                        drain_timeout: Duration::from_secs(2),
                    };
                    if args.terminal {
                        ProcessSession::spawn_terminal(
                            spec,
                            crate::agent::process::TerminalSize {
                                rows: 24,
                                columns: 80,
                            },
                            limits,
                        )
                    } else {
                        ProcessSession::spawn(spec, limits)
                    }
                })
            })
            .await
            .map_err(|_| RunError::EventSink)?
            .map_err(|_| RunError::EventSink)
        })
    }
}
fn setup(root: PathBuf) -> (ShellTools, Arc<Host>) {
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
        root,
        allowed: AtomicBool::new(true),
        launches: AtomicUsize::new(0),
        selected_scope: Mutex::new(None),
    });
    (ShellTools::new(registry, scope, host.clone()), host)
}

#[tokio::test]
async fn result_ack_retains_its_original_lease_after_owner_selects_successor() {
    let temp = tempfile::tempdir().unwrap();
    let (tools, host) = setup(temp.path().into());
    let first = call(
        "first-scope",
        EXEC,
        json!({"command":"printf first", "yield_ms":5000}),
    );
    let result = tools.authorize_and_execute(first.clone()).await.unwrap();
    let id: Uuid = serde_json::from_value(
        serde_json::from_str::<Value>(&result.content).unwrap()["processId"].clone(),
    )
    .unwrap();
    let mut next = tools.scope.clone();
    next.lease_id = Uuid::new_v4();
    *host.selected_scope.lock().unwrap() = Some(next.clone());
    // Completion belongs to the lease under which the process was launched,
    // even if the owner has already advanced before the result is committed.
    tools.result_committed(first).await.unwrap();
    assert!(matches!(
        tools.registry.poll(&tools.scope, id, Duration::ZERO).await,
        Err(ProcessRegistryError::NotFound)
    ));
    let second = call(
        "second-scope",
        EXEC,
        json!({"command":"printf second", "yield_ms":5000}),
    );
    let result = tools.authorize_and_execute(second.clone()).await.unwrap();
    let id: Uuid = serde_json::from_value(
        serde_json::from_str::<Value>(&result.content).unwrap()["processId"].clone(),
    )
    .unwrap();
    assert!(matches!(
        tools.registry.poll(&next, id, Duration::ZERO).await,
        Ok(ProcessPoll::Finished(_))
    ));
    assert!(
        tools
            .registry
            .poll(&tools.scope, id, Duration::ZERO)
            .await
            .is_err()
    );
    tools.result_committed(second).await.unwrap();
    let mut foreign = next;
    foreign.turn_id = Uuid::new_v4();
    *host.selected_scope.lock().unwrap() = Some(foreign);
    assert!(matches!(
        tools
            .authorize_and_execute(call("foreign", EXEC, json!({"command":"true"})))
            .await,
        Err(RunError::ToolRejected)
    ));
    assert_eq!(host.launches.load(Ordering::SeqCst), 2);
    tools.registry.shutdown().await.unwrap();
}
fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: id.into(),
        name: name.into(),
        arguments,
    }
}
#[tokio::test]
async fn failed_result_ack_retains_owner_and_process_identity_without_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let (tools, _) = setup(temp.path().into());
    let process = Uuid::new_v4();
    let call = call(
        "call-ack",
        EXEC,
        json!({"command":"private fixture command"}),
    );
    tools.calls.lock().unwrap().insert(
        call.call_id.clone(),
        Receipt {
            call: call.clone(),
            scope: tools.scope.clone(),
            forget: Some(process),
            delivered: true,
            acknowledged: false,
        },
    );
    let Err(RunError::EventSinkDiagnostic(detail)) = tools.result_committed(call).await else {
        panic!("the original process owner diagnostic is required");
    };
    let value = serde_json::to_value(detail).unwrap();
    assert_eq!(value["code"], "SHELL_RESULT_ACK_UNCONFIRMED");
    assert_eq!(value["safeCause"], "process_not_found");
    assert_eq!(value["callId"], "call-ack");
    assert_eq!(value["resultId"], process.to_string());
    assert_eq!(
        value["conversationId"],
        tools.scope.conversation_id.to_string()
    );
    assert!(!value.to_string().contains("private fixture command"));
    assert!(!tools.calls.lock().unwrap()["call-ack"].acknowledged);
}
#[tokio::test]
async fn process_result_is_retained_until_commit_and_duplicate_calls_never_launch() {
    let temp = tempfile::tempdir().unwrap();
    let (tools, host) = setup(temp.path().into());
    let exec = call(
        "exec",
        EXEC,
        json!({"command":"printf kept; exit 7","yield_ms":5000}),
    );
    let result = tools.authorize_and_execute(exec.clone()).await.unwrap();
    assert!(result.is_error);
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["state"], "finished");
    assert_eq!(value["exitCode"], 7);
    let id: Uuid = serde_json::from_value(value["processId"].clone()).unwrap();
    assert!(matches!(
        tools
            .registry
            .poll(&tools.scope, id, Duration::ZERO)
            .await
            .unwrap(),
        ProcessPoll::Finished(_)
    ));
    assert!(tools.authorize_and_execute(exec.clone()).await.is_err());
    tools.result_committed(exec.clone()).await.unwrap();
    tools.result_committed(exec.clone()).await.unwrap();
    assert!(matches!(
        tools.registry.poll(&tools.scope, id, Duration::ZERO).await,
        Err(ProcessRegistryError::NotFound)
    ));
    assert!(tools.authorize_and_execute(exec).await.is_err());
    assert_eq!(host.launches.load(Ordering::SeqCst), 1);
    tools.registry.shutdown().await.unwrap();
}
#[tokio::test]
async fn yielded_command_accepts_input_and_rechecks_authorization() {
    let temp = tempfile::tempdir().unwrap();
    let (tools, host) = setup(temp.path().into());
    let exec = call(
        "exec",
        EXEC,
        json!({"command":"read value; printf '%s' \"$value\""}),
    );
    let result = tools.authorize_and_execute(exec.clone()).await.unwrap();
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["state"], "running");
    let id = value["processId"].clone();
    tools.result_committed(exec).await.unwrap();
    let write = call(
        "write",
        PROCESS,
        json!({"action":"write","process_id":id,"text":"hello\n"}),
    );
    host.allowed.store(false, Ordering::SeqCst);
    assert!(tools.authorize_and_execute(write.clone()).await.is_err());
    host.allowed.store(true, Ordering::SeqCst);
    let result = tools.authorize_and_execute(write.clone()).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&result.content).unwrap()["writtenBytes"],
        6
    );
    tools.result_committed(write.clone()).await.unwrap();
    assert!(tools.authorize_and_execute(write).await.is_err());
    let poll = call(
        "poll",
        PROCESS,
        json!({"action":"poll","process_id":id,"yield_ms":5000}),
    );
    let result = tools.authorize_and_execute(poll.clone()).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("hello"));
    tools.result_committed(poll).await.unwrap();
    tools.registry.shutdown().await.unwrap();
}
#[test]
fn model_arguments_cannot_supply_host_scope_or_unbounded_waits() {
    for args in [
        json!({"command":"true","directory":"/"}),
        json!({"command":"true","yield_ms":60001}),
        json!({"command":"\u{0}"}),
    ] {
        assert!(ShellTools::parse(&call("exec", EXEC, args)).is_err());
    }
}

#[tokio::test]
async fn terminal_output_is_combined_and_cancel_requires_final_poll() {
    let temp = tempfile::tempdir().unwrap();
    let (tools, _) = setup(temp.path().into());
    let exec = call(
        "tty",
        EXEC,
        json!({"command":"test -t 0 && printf terminal; read value", "terminal":true,"yield_ms":10}),
    );
    let result = tools.authorize_and_execute(exec.clone()).await.unwrap();
    let value: Value = serde_json::from_str(&result.content).unwrap();
    let id = value["processId"].clone();
    assert_eq!(value["state"], "running");
    assert!(value["terminal"].is_object());
    tools.result_committed(exec).await.unwrap();
    let cancel = call(
        "cancel",
        PROCESS,
        json!({"action":"cancel","process_id":id}),
    );
    let result = tools.authorize_and_execute(cancel.clone()).await.unwrap();
    assert!(result.content.contains("requested"));
    tools.result_committed(cancel).await.unwrap();
    let poll = call(
        "final",
        PROCESS,
        json!({"action":"poll","process_id":id,"yield_ms":5000}),
    );
    let result = tools.authorize_and_execute(poll.clone()).await.unwrap();
    assert!(result.is_error);
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["state"], "cancelled");
    assert_eq!(value["stdout"]["head"], "");
    assert!(value["terminal"].is_object());
    tools.result_committed(poll).await.unwrap();
    tools.registry.shutdown().await.unwrap();
}
