use super::*;
use crate::agent::process::SupervisedProcess;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Launcher {
    approved: AtomicBool,
    launches: AtomicUsize,
}
impl StopHookLauncher for Launcher {
    fn launch<'a>(
        &'a self,
        _: &'a StopRequest,
        process: ProcessSpec,
        limits: ProcessLimits,
    ) -> HookLaunchFuture<'a> {
        Box::pin(async move {
            if !self.approved.load(Ordering::SeqCst) {
                return Err(HookCommandError);
            }
            self.launches.fetch_add(1, Ordering::SeqCst);
            SupervisedProcess::spawn(process, limits).map_err(|_| HookCommandError)
        })
    }
}
struct Source {
    script: String,
    root: PathBuf,
    launcher: Arc<Launcher>,
}
impl StopCommandSource for Source {
    fn prepare(&self, _: &StopRequest) -> Result<Option<AuthorizedStopCommand>, HookCommandError> {
        Ok(Some(AuthorizedStopCommand {
            process: ProcessSpec {
                program: "/bin/sh".into(),
                arguments: vec!["-c".into(), self.script.clone().into()],
                directory: self.root.clone(),
                environment: Default::default(),
            },
            transcript_path: None,
            launcher: self.launcher.clone(),
        }))
    }
}
fn request(target: StopTarget) -> StopRequest {
    StopRequest {
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "model".into(),
            revision: None,
        },
        permission_mode: "default".into(),
        stop_hook_active: true,
        last_assistant_message: Some("last answer".into()),
        target,
    }
}
fn setup(
    root: &Path,
    script: &str,
    limit: usize,
    timeout: Duration,
) -> (ProcessStopHook, Arc<Launcher>) {
    let launcher = Arc::new(Launcher {
        approved: AtomicBool::new(true),
        launches: AtomicUsize::new(0),
    });
    let source = Source {
        script: script.into(),
        root: root.into(),
        launcher: launcher.clone(),
    };
    (
        ProcessStopHook::new(Arc::new(source), timeout, limit).unwrap(),
        launcher,
    )
}

#[cfg(unix)]
#[test]
fn invalid_utf8_path_uses_reference_lossy_display() {
    use std::os::unix::ffi::OsStringExt;
    let root = tempfile::tempdir().unwrap();
    let launcher = Arc::new(Launcher {
        approved: AtomicBool::new(true),
        launches: AtomicUsize::new(0),
    });
    let source = Source {
        script: String::new(),
        root: root.path().into(),
        launcher,
    };
    let request = request(StopTarget::Stop);
    let mut plan = source.prepare(&request).unwrap().unwrap();
    plan.transcript_path = Some(PathBuf::from(std::ffi::OsString::from_vec(vec![0xff])));
    let input: serde_json::Value =
        serde_json::from_slice(&command_input(&request, &plan).unwrap()).unwrap();
    assert_eq!(input["transcript_path"], "\u{fffd}");
}

#[tokio::test]
async fn subprocess_receives_stop_input_and_exit_two_feedback() {
    let root = tempfile::tempdir().unwrap();
    let (hook, _) = setup(
        root.path(),
        "cat > input.json; printf ignored; printf '  retry tests  ' >&2; exit 2",
        1024,
        Duration::from_secs(2),
    );
    let request = request(StopTarget::Stop);
    let result = hook.run(&request).await;
    assert_eq!(
        result.status,
        StopHandlerStatus::Blocked {
            feedback: "retry tests".into()
        }
    );
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("input.json")).unwrap()).unwrap();
    assert_eq!(input["session_id"], request.conversation_id.to_string());
    assert_eq!(input["turn_id"], request.turn_id.to_string());
    assert_eq!(input["hook_event_name"], "Stop");
    assert_eq!(input["model"], "model");
    assert_eq!(input["permission_mode"], "default");
    assert_eq!(input["stop_hook_active"], true);
    assert_eq!(input["last_assistant_message"], "last answer");
    assert!(input["transcript_path"].is_null());
    assert!(input.get("agent_id").is_none());
}

#[tokio::test]
async fn subagent_process_receives_identity_and_structured_decision() {
    let root = tempfile::tempdir().unwrap();
    let (hook, _) = setup(
        root.path(),
        "cat > input.json; printf '%s' '{\"decision\":\"block\",\"reason\":\"continue child\"}'",
        1024,
        Duration::from_secs(2),
    );
    let request = request(StopTarget::SubagentStop {
        agent_id: "child-1".into(),
        agent_type: "research".into(),
        agent_transcript_path: None,
    });
    assert_eq!(
        hook.run(&request).await.status,
        StopHandlerStatus::Blocked {
            feedback: "continue child".into()
        }
    );
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("input.json")).unwrap()).unwrap();
    assert_eq!(input["hook_event_name"], "SubagentStop");
    assert_eq!(input["agent_id"], "child-1");
    assert_eq!(input["agent_type"], "research");
    assert!(input["agent_transcript_path"].is_null());
}

#[tokio::test]
async fn launch_denial_timeout_and_incomplete_relevant_stream_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let request = request(StopTarget::Stop);
    let (denied, launcher) = setup(root.path(), "cat >/dev/null", 32, Duration::from_secs(2));
    launcher.approved.store(false, Ordering::SeqCst);
    assert!(matches!(
        denied.run(&request).await.status,
        StopHandlerStatus::Failed { .. }
    ));
    assert_eq!(launcher.launches.load(Ordering::SeqCst), 0);
    for script in [
        "cat >/dev/null; sleep 2",
        "cat >/dev/null; printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'",
        "cat >/dev/null; printf '\\377'",
        "cat >/dev/null; printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' >&2; exit 2",
    ] {
        let (hook, _) = setup(root.path(), script, 16, Duration::from_millis(100));
        assert!(
            matches!(
                hook.run(&request).await.status,
                StopHandlerStatus::Failed { .. }
            ),
            "{script}"
        );
    }
}

#[tokio::test]
async fn irrelevant_truncated_stdout_does_not_hide_exit_two_feedback() {
    let root = tempfile::tempdir().unwrap();
    let (hook, _) = setup(
        root.path(),
        "cat >/dev/null; printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; printf retry >&2; exit 2",
        16,
        Duration::from_secs(2),
    );
    assert_eq!(
        hook.run(&request(StopTarget::Stop)).await.status,
        StopHandlerStatus::Blocked {
            feedback: "retry".into()
        }
    );
}

#[tokio::test]
async fn exit_two_feedback_uses_lossy_utf8_and_accepts_closed_stdin() {
    let root = tempfile::tempdir().unwrap();
    let (hook, _) = setup(
        root.path(),
        "exec 0<&-; printf 'retry\\377' >&2; exit 2",
        1024,
        Duration::from_secs(2),
    );
    let mut request = request(StopTarget::Stop);
    request.last_assistant_message = Some("x".repeat(1024 * 1024));
    assert_eq!(
        hook.run(&request).await.status,
        StopHandlerStatus::Blocked {
            feedback: "retry\u{fffd}".into()
        }
    );
}

#[tokio::test]
async fn structured_stdout_uses_reference_lossy_utf8() {
    let root = tempfile::tempdir().unwrap();
    let (hook, _) = setup(
        root.path(),
        "cat >/dev/null; printf '{\"decision\":\"block\",\"reason\":\"retry\\377\"}'",
        1024,
        Duration::from_secs(2),
    );
    assert_eq!(
        hook.run(&request(StopTarget::Stop)).await.status,
        StopHandlerStatus::Blocked {
            feedback: "retry\u{fffd}".into()
        }
    );
}

#[tokio::test]
async fn dropping_stop_hook_terminates_its_owned_process() {
    let root = tempfile::tempdir().unwrap();
    let (hook, _) = setup(
        root.path(),
        "cat >/dev/null; printf '%s' \"$$\" > pid; exec /bin/sleep 30",
        1024,
        Duration::from_secs(40),
    );
    let request = request(StopTarget::Stop);
    let mut running = Box::pin(hook.run(&request));
    let pid = tokio::select! {
        _ = &mut running => panic!("hook ended before cancellation"),
        pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(value) = std::fs::read_to_string(root.path().join("pid")) && let Ok(pid) = value.parse::<i32>() { break pid; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }) => pid.unwrap(),
    };
    drop(running);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
                == Err(nix::errno::Errno::ESRCH)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
