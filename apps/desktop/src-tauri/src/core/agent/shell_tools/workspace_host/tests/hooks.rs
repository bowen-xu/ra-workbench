use super::*;
use crate::agent::compaction_hook_process::*;
use research_agent_core::{
    context::{compaction_hooks::*, compaction_strategy::CompactionPhase},
    protocol::ModelIdentity,
};

struct Source {
    directory: PathBuf,
    script: String,
    launcher: Arc<dyn HookProcessLauncher>,
}
impl CompactionHookCommandSource for Source {
    fn prepare(
        &self,
        _: &CompactionHookRequest,
    ) -> Result<Option<AuthorizedHookCommand>, HookCommandError> {
        Ok(Some(AuthorizedHookCommand {
            process: WorkspaceShellHost::spec(self.directory.clone(), self.script.clone()),
            transcript_path: None,
            launcher: Some(self.launcher.clone()),
        }))
    }
}
async fn fixture(
    script: &str,
) -> (
    tempfile::TempDir,
    Arc<WorkspaceShellHost>,
    PathBuf,
    CompactionHookRequest,
    ProcessCompactionHooks,
    Arc<AtomicBool>,
) {
    let temp = tempfile::tempdir().unwrap();
    let scope = scope();
    let host = setup(
        temp.path().canonicalize().unwrap(),
        Arc::new(ProcessRegistry::default()),
        scope.clone(),
        Arc::new(AtomicBool::new(true)),
    )
    .await;
    let directory = host
        .inspect_owned(|lease| Ok(lease.working_directory().unwrap()))
        .await
        .unwrap();
    let request = CompactionHookRequest {
        conversation_id: scope.conversation_id,
        turn_id: scope.turn_id,
        operation_id: Uuid::new_v4(),
        stage: CompactionHookStage::SessionStartCompact,
        phase: CompactionPhase::Manual,
        model: ModelIdentity {
            provider_id: "test".into(),
            model_id: "test-model".into(),
            revision: None,
        },
    };
    let allowed = Arc::new(AtomicBool::new(true));
    let check = allowed.clone();
    let model = request.model.clone();
    let launcher = host.hook_launcher(Arc::new(move |_, request| {
        if check.load(Ordering::SeqCst) && request.model == model {
            Ok(())
        } else {
            Err(HookCommandError)
        }
    }));
    let hooks = ProcessCompactionHooks::new(
        Arc::new(Source {
            directory: directory.clone(),
            script: script.into(),
            launcher,
        }),
        Duration::from_secs(5),
        1024,
    )
    .unwrap();
    (temp, host, directory, request, hooks, allowed)
}
async fn started(directory: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !directory.join("running").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hook_uses_original_lease_and_releases_intent_after_actual_exit() {
    let (_temp, host, directory, request, hooks, _) = fixture("cat >/dev/null; touch running; while test ! -f release; do sleep 0.01; done; printf '%s' '{\"hookSpecificOutput\":{\"hookEventName\":\"SessionStart\",\"additionalContext\":\"restored\"}}'").await;
    let run = tokio::spawn(async move { hooks.run_with_context(request).await });
    started(&directory).await;
    assert!(host.changes().await.is_err());
    std::fs::write(directory.join("release"), "go").unwrap();
    let result = run.await.unwrap();
    assert_eq!(result.outcome, CompactionHookOutcome::Continue);
    assert_eq!(result.additional_contexts, ["restored"]);
    assert!(host.changes().await.is_ok());
    assert!(!directory.parent().unwrap().join("execution.json").exists());
}

#[tokio::test]
async fn changed_marker_blocks_context_and_preserves_uncertain_workspace() {
    let (_temp, host, directory, request, hooks, _) = fixture("cat >/dev/null; touch running; while test ! -f release; do sleep 0.01; done; printf restored").await;
    let run = tokio::spawn(async move { hooks.run_with_context(request).await });
    started(&directory).await;
    let marker = directory.parent().unwrap().join("execution.json");
    std::fs::rename(&marker, marker.with_extension("old")).unwrap();
    std::fs::write(&marker, "replacement").unwrap();
    std::fs::write(directory.join("release"), "go").unwrap();
    let result = run.await.unwrap();
    assert_eq!(result.outcome, CompactionHookOutcome::Failed);
    assert!(result.additional_contexts.is_empty());
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "replacement");
    assert!(host.changes().await.is_err());
}

#[tokio::test]
async fn revoked_or_wrong_turn_cannot_create_writer_intent_or_run() {
    let (_temp, host, directory, mut request, hooks, allowed) =
        fixture("cat >/dev/null; touch running").await;
    request.turn_id = Uuid::new_v4();
    assert_eq!(
        hooks.run_with_context(request.clone()).await.outcome,
        CompactionHookOutcome::Failed
    );
    request.turn_id = host.scope.turn_id;
    allowed.store(false, Ordering::SeqCst);
    assert_eq!(
        hooks.run_with_context(request).await.outcome,
        CompactionHookOutcome::Failed
    );
    assert!(!directory.join("running").exists());
    assert!(!directory.parent().unwrap().join("execution.json").exists());
    assert!(host.changes().await.unwrap().is_empty());
}

#[tokio::test]
async fn canceled_blocking_admission_cannot_launch_after_policy_returns() {
    let (_temp, host, directory, request, _, _) = fixture("unused").await;
    let (entered, ready) = tokio::sync::oneshot::channel();
    let entered = Mutex::new(Some(entered));
    let (release, wait) = std::sync::mpsc::channel();
    let wait = Mutex::new(wait);
    let completed = Arc::new(tokio::sync::Notify::new());
    let done = completed.clone();
    let launcher = host.hook_launcher(Arc::new(move |_, _| {
        entered.lock().unwrap().take().unwrap().send(()).unwrap();
        wait.lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        done.notify_one();
        Ok(())
    }));
    let hooks = ProcessCompactionHooks::new(
        Arc::new(Source {
            directory: directory.clone(),
            script: "touch running".into(),
            launcher,
        }),
        Duration::from_secs(5),
        1024,
    )
    .unwrap();
    let run = tokio::spawn(async move { hooks.run_with_context(request).await });
    ready.await.unwrap();
    run.abort();
    assert!(run.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    completed.notified().await;
    assert!(!directory.join("running").exists());
    assert!(!directory.parent().unwrap().join("execution.json").exists());
    assert!(host.changes().await.unwrap().is_empty());
}
