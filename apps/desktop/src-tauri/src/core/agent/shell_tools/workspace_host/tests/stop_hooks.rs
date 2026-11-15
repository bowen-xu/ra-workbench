use super::*;
use crate::agent::{
    compaction_hook_process::HookCommandError,
    stop_hook_process::{
        AuthorizedStopCommand, ProcessStopHook, StopCommandSource, StopHookLauncher, StopRequest,
        StopTarget,
    },
};
use research_agent_core::{protocol::ModelIdentity, runner::StopHandlerStatus};

struct Source {
    directory: PathBuf,
    script: String,
    launcher: Arc<dyn StopHookLauncher>,
}
impl StopCommandSource for Source {
    fn prepare(&self, _: &StopRequest) -> Result<Option<AuthorizedStopCommand>, HookCommandError> {
        Ok(Some(AuthorizedStopCommand {
            process: WorkspaceShellHost::spec(self.directory.clone(), self.script.clone()),
            transcript_path: None,
            launcher: self.launcher.clone(),
        }))
    }
}
async fn fixture(
    script: &str,
) -> (
    tempfile::TempDir,
    Arc<WorkspaceShellHost>,
    PathBuf,
    StopRequest,
    ProcessStopHook,
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
    let request = StopRequest {
        conversation_id: scope.conversation_id,
        turn_id: scope.turn_id,
        model: ModelIdentity {
            provider_id: "test".into(),
            model_id: "model".into(),
            revision: None,
        },
        permission_mode: "default".into(),
        stop_hook_active: false,
        last_assistant_message: None,
        target: StopTarget::Stop,
    };
    let allowed = Arc::new(AtomicBool::new(true));
    let check = allowed.clone();
    let model = request.model.clone();
    let launcher = host.stop_hook_launcher(Arc::new(move |_, request| {
        if check.load(Ordering::SeqCst) && request.model == model {
            Ok(())
        } else {
            Err(HookCommandError)
        }
    }));
    let hook = ProcessStopHook::new(
        Arc::new(Source {
            directory: directory.clone(),
            script: script.into(),
            launcher,
        }),
        Duration::from_secs(5),
        1024,
    )
    .unwrap();
    (temp, host, directory, request, hook, allowed)
}

#[tokio::test]
async fn stop_hook_uses_original_sandbox_and_retains_lease_until_exit() {
    let (temp, host, directory, request, hook, _) = fixture("cat >/dev/null; touch running; while test ! -f release; do sleep 0.01; done; if printf corrupt > ../../../originals/keep.txt; then exit 4; fi; printf '{\"decision\":\"block\",\"reason\":\"retry\"}'").await;
    let run = tokio::spawn(async move { hook.run(&request).await });
    tokio::time::timeout(Duration::from_secs(4), async {
        while !directory.join("running").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(host.changes().await.is_err());
    std::fs::write(directory.join("release"), "go").unwrap();
    assert_eq!(
        run.await.unwrap().status,
        StopHandlerStatus::Blocked {
            feedback: "retry".into()
        }
    );
    assert!(host.changes().await.is_ok());
    assert!(!directory.parent().unwrap().join("execution.json").exists());
    assert_eq!(
        std::fs::read_to_string(temp.path().join("originals/keep.txt")).unwrap(),
        "original"
    );
}

#[tokio::test]
async fn changed_writer_marker_discards_stop_feedback_and_retains_uncertainty() {
    let (_temp, host, directory, request, hook, _) = fixture("cat >/dev/null; touch running; while test ! -f release; do sleep 0.01; done; printf '{\"decision\":\"block\",\"reason\":\"retry\"}'").await;
    let run = tokio::spawn(async move { hook.run(&request).await });
    tokio::time::timeout(Duration::from_secs(4), async {
        while !directory.join("running").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let marker = directory.parent().unwrap().join("execution.json");
    std::fs::rename(&marker, marker.with_extension("old")).unwrap();
    std::fs::write(&marker, "replacement").unwrap();
    std::fs::write(directory.join("release"), "go").unwrap();
    assert!(matches!(
        run.await.unwrap().status,
        StopHandlerStatus::Failed { .. }
    ));
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "replacement");
    assert!(host.changes().await.is_err());
}

#[tokio::test]
async fn wrong_turn_and_revoked_stop_grant_do_not_launch_or_hold_writer() {
    let (_temp, host, directory, mut request, hook, allowed) =
        fixture("cat >/dev/null; touch running").await;
    request.turn_id = Uuid::new_v4();
    assert!(matches!(
        hook.run(&request).await.status,
        StopHandlerStatus::Failed { .. }
    ));
    request.turn_id = host.scope.turn_id;
    allowed.store(false, Ordering::SeqCst);
    assert!(matches!(
        hook.run(&request).await.status,
        StopHandlerStatus::Failed { .. }
    ));
    assert!(!directory.join("running").exists());
    assert!(!directory.parent().unwrap().join("execution.json").exists());
    assert!(host.changes().await.unwrap().is_empty());
}

#[tokio::test]
async fn prepared_workspace_requires_both_native_and_handler_authorization() {
    let (_temp, host, directory, request, _, _) = fixture("unused").await;
    let native = Arc::new(AtomicBool::new(true));
    let handler = Arc::new(AtomicBool::new(true));
    let native_check = native.clone();
    let workspace = crate::agent::compaction_hook_config::native::HookWorkspace::from_host(
        host.as_ref().clone(),
        Arc::new(move |_| {
            if native_check.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(HookCommandError)
            }
        }),
    )
    .await
    .unwrap();
    let handler_check = handler.clone();
    let launcher = workspace.stop_launcher(Arc::new(move |_, _| {
        if handler_check.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(HookCommandError)
        }
    }));
    let hook = ProcessStopHook::new(
        Arc::new(Source {
            directory: directory.clone(),
            script: "cat >/dev/null; touch running".into(),
            launcher,
        }),
        Duration::from_secs(5),
        1024,
    )
    .unwrap();
    native.store(false, Ordering::SeqCst);
    assert!(matches!(
        hook.run(&request).await.status,
        StopHandlerStatus::Failed { .. }
    ));
    native.store(true, Ordering::SeqCst);
    handler.store(false, Ordering::SeqCst);
    assert!(matches!(
        hook.run(&request).await.status,
        StopHandlerStatus::Failed { .. }
    ));
    assert!(!directory.join("running").exists());
    handler.store(true, Ordering::SeqCst);
    assert_eq!(
        hook.run(&request).await.status,
        StopHandlerStatus::Completed
    );
    assert!(directory.join("running").exists());
}

#[tokio::test]
async fn cancellation_during_stop_authorization_prevents_late_process_launch() {
    let (_temp, host, directory, request, _, _) = fixture("unused").await;
    let (entered, ready) = tokio::sync::oneshot::channel();
    let entered = Mutex::new(Some(entered));
    let (release, wait) = std::sync::mpsc::channel();
    let wait = Mutex::new(wait);
    let completed = Arc::new(tokio::sync::Notify::new());
    let done = completed.clone();
    let launcher = host.stop_hook_launcher(Arc::new(move |_, _| {
        entered.lock().unwrap().take().unwrap().send(()).unwrap();
        wait.lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        done.notify_one();
        Ok(())
    }));
    let hook = ProcessStopHook::new(
        Arc::new(Source {
            directory: directory.clone(),
            script: "touch running".into(),
            launcher,
        }),
        Duration::from_secs(5),
        1024,
    )
    .unwrap();
    let run = tokio::spawn(async move { hook.run(&request).await });
    ready.await.unwrap();
    run.abort();
    assert!(run.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    completed.notified().await;
    assert!(!directory.join("running").exists());
    assert!(!directory.parent().unwrap().join("execution.json").exists());
    assert!(host.changes().await.unwrap().is_empty());
}
