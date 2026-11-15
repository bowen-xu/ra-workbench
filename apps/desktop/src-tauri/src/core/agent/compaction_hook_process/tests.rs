use super::*;
use research_agent_core::protocol::ModelIdentity;
use std::path::Path;
use uuid::Uuid;

#[path = "tests/concurrent.rs"]
mod concurrent;

struct Source {
    script: String,
    directory: PathBuf,
}
impl CompactionHookCommandSource for Source {
    fn prepare(
        &self,
        _: &CompactionHookRequest,
    ) -> Result<Option<AuthorizedHookCommand>, HookCommandError> {
        Ok(Some(AuthorizedHookCommand {
            process: ProcessSpec {
                program: "/bin/sh".into(),
                arguments: vec!["-c".into(), self.script.clone().into()],
                directory: self.directory.clone(),
                environment: Default::default(),
            },
            transcript_path: None,
            launcher: None,
        }))
    }
}

fn request(stage: CompactionHookStage) -> CompactionHookRequest {
    CompactionHookRequest {
        operation_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "test".into(),
            model_id: "test-model".into(),
            revision: None,
        },
        phase: CompactionPhase::Manual,
        stage,
    }
}

fn hooks(root: &Path, script: &str, limit: usize, timeout: Duration) -> ProcessCompactionHooks {
    ProcessCompactionHooks::new(
        Arc::new(Source {
            script: script.into(),
            directory: root.to_owned(),
        }),
        timeout,
        limit,
    )
    .unwrap()
}

#[tokio::test]
async fn native_hook_receives_stdin_and_returns_stop_context_without_stderr() {
    let root = tempfile::tempdir().unwrap();
    let hooks = hooks(
        root.path(),
        "cat > input.json; printf 'ignored diagnostic' >&2; printf '%s' '{\"continue\":false,\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"trusted process context\"}}'",
        1024,
        Duration::from_secs(2),
    );
    let request = request(CompactionHookStage::SessionStartCompact);
    let result = hooks.run_with_context(request.clone()).await;
    assert_eq!(result.outcome, CompactionHookOutcome::Stop);
    assert_eq!(result.additional_contexts, ["trusted process context"]);
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("input.json")).unwrap()).unwrap();
    assert_eq!(input["session_id"], request.conversation_id.to_string());
    assert_eq!(input["turn_id"], request.turn_id.to_string());
    assert_eq!(input["model"], "test-model");
    assert_eq!(input["hook_event_name"], "SessionStart");
    assert_eq!(input["source"], "compact");
    assert!(input.get("trigger").is_none());
}

#[tokio::test]
async fn native_hook_failure_timeout_truncation_and_invalid_utf8_never_create_context() {
    let root = tempfile::tempdir().unwrap();
    for script in [
        "cat >/dev/null; printf 'context'; exit 1",
        "cat >/dev/null; printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'",
        "cat >/dev/null; printf '\\377'",
        "cat >/dev/null; sleep 2",
    ] {
        let hooks = hooks(root.path(), script, 16, Duration::from_millis(100));
        let result = hooks
            .run_with_context(request(CompactionHookStage::SessionStartCompact))
            .await;
        assert_eq!(result.outcome, CompactionHookOutcome::Failed, "{script}");
        assert!(result.additional_contexts.is_empty());
    }
}

#[tokio::test]
async fn native_precompact_has_manual_trigger_and_no_context_output() {
    let root = tempfile::tempdir().unwrap();
    let hooks = hooks(
        root.path(),
        "cat > input.json; printf '%s' '{\"decision\":\"block\"}'",
        1024,
        Duration::from_secs(2),
    );
    let result = hooks
        .run_with_context(request(CompactionHookStage::PreCompact))
        .await;
    assert_eq!(result.outcome, CompactionHookOutcome::Continue);
    assert!(result.additional_contexts.is_empty());
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("input.json")).unwrap()).unwrap();
    assert_eq!(input["trigger"], "manual");
    assert_eq!(input["hook_event_name"], "PreCompact");
    assert!(input.get("source").is_none());
}

#[tokio::test]
async fn dropping_native_hook_future_terminates_its_process() {
    let root = tempfile::tempdir().unwrap();
    let hooks = hooks(
        root.path(),
        "printf '%s' \"$$\" > pid; exec /bin/sleep 30",
        1024,
        Duration::from_secs(40),
    );
    let mut running = hooks.run_with_context(request(CompactionHookStage::SessionStartCompact));
    let pid = tokio::select! {
        _ = &mut running => panic!("hook ended before cancellation"),
        pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(value) = std::fs::read_to_string(root.path().join("pid"))
                    && let Ok(pid) = value.parse::<i32>() {
                    break pid;
                }
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

#[tokio::test]
async fn stdin_failure_terminates_hook_without_waiting_for_its_execution_deadline() {
    let root = tempfile::tempdir().unwrap();
    let hooks = hooks(
        root.path(),
        "printf '%s' \"$$\" > pid; exec 0<&-; exec /bin/sleep 30",
        1024,
        Duration::from_secs(40),
    );
    let mut request = request(CompactionHookStage::SessionStartCompact);
    // macOS can grow the pipe to 64 KiB. Fill the allowed JSON size plus
    // its newline so a command which never reads cannot accept it all.
    request.model.model_id.clear();
    let plan = hooks.source.prepare(&request).unwrap().unwrap();
    let overhead = command_input(&request, &plan).unwrap().len();
    request.model.model_id = "m".repeat(64 * 1024 + 1 - overhead);
    assert_eq!(command_input(&request, &plan).unwrap().len(), 64 * 1024 + 1);
    let result = tokio::time::timeout(Duration::from_secs(2), hooks.run_with_context(request))
        .await
        .expect("stdin failure must cancel the owned process immediately");
    assert_eq!(result.outcome, CompactionHookOutcome::Failed);
    assert!(result.additional_contexts.is_empty());
    let pid: i32 = std::fs::read_to_string(root.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
}
