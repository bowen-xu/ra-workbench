use super::*;
use std::{
    sync::{Mutex, mpsc},
    time::Duration,
};
struct PausedAuthority {
    inner: Authority,
    release: Mutex<mpsc::Receiver<()>>,
    entered: tokio::sync::Notify,
    pause_command: bool,
}
struct WorkerObserver(Arc<tokio::sync::Notify>);
impl HookHandlerObserver for WorkerObserver {
    fn notify(&self, _: Uuid, _: &CompactionHookRequest, _: CompactionHookNotification) {}
}
impl Drop for WorkerObserver {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}
impl ConfigurationAuthority for PausedAuthority {
    fn authorize_source(
        &self,
        root: &InstructionRootIdentity<'_>,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<(), HookConfigError> {
        self.inner.authorize_source(root, host, record)?;
        if self.pause_command {
            return Ok(());
        }
        self.entered.notify_one();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| HookConfigError)
    }
    fn authorize_command(
        &self,
        root: &InstructionRootIdentity<'_>,
        hook: &ConfiguredHook,
        revision: &str,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<AuthorizedHookCommand, HookConfigError> {
        let result = self
            .inner
            .authorize_command(root, hook, revision, host, record)?;
        if self.pause_command {
            self.entered.notify_one();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| HookConfigError)?;
        }
        Ok(result)
    }
}
impl HookInvocationAuthority for PausedAuthority {
    fn check(
        &self,
        id: Uuid,
        revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        self.inner.check(id, revision, request)
    }
}
#[tokio::test]
async fn canceled_source_preparation_never_grants_commands_or_returns_hooks() {
    canceled_preparation(false).await;
}

#[tokio::test]
async fn canceled_command_preparation_stops_before_next_grant_and_never_publishes_hooks() {
    canceled_preparation(true).await;
}

async fn canceled_preparation(pause_command: bool) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("agent")).unwrap();
    let first = text(&root);
    let text = format!(
        "{first}{}",
        first.replace("schema_version=1\n", "").replace(
            "00000000-0000-0000-0000-000000000012",
            "00000000-0000-0000-0000-000000000013"
        )
    );
    std::fs::write(root.join("agent/hooks.toml"), &text).unwrap();
    let record = record();
    let (release, blocked) = mpsc::channel();
    let authority = Arc::new(PausedAuthority {
        inner: Authority {
            root: root.clone(),
            record: record.clone(),
            revision: parse(&text).unwrap().revision().into(),
            source_allowed: AtomicBool::new(true),
            invocation_allowed: AtomicBool::new(true),
            grants: AtomicUsize::new(0),
            checks: AtomicUsize::new(0),
        },
        release: Mutex::new(blocked),
        entered: tokio::sync::Notify::new(),
        pause_command,
    });
    let finished = Arc::new(tokio::sync::Notify::new());
    let factory = ConfiguredHookFactory::new(
        root.clone(),
        authority.clone(),
        Arc::new(WorkerObserver(finished.clone())),
    )
    .unwrap();
    let preparation = tokio::spawn(async move { factory.prepare(AgentHost::Notes, &record).await });
    tokio::time::timeout(Duration::from_secs(5), authority.entered.notified())
        .await
        .unwrap();
    preparation.abort();
    assert!(matches!(preparation.await, Err(error) if error.is_cancelled()));
    release.send(()).unwrap();
    // Last observer drop proves the detached blocking worker has finished;
    // assertions do not race its callbacks or rely on a timed sleep.
    tokio::time::timeout(Duration::from_secs(5), finished.notified())
        .await
        .unwrap();
    assert_eq!(
        authority.inner.grants.load(Ordering::SeqCst),
        usize::from(pause_command),
        "an entered callback may finish, but no subsequent grant may begin"
    );
    assert_eq!(authority.inner.checks.load(Ordering::SeqCst), 0);
    assert!(!root.join("executed").exists());
}
