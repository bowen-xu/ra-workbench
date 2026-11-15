//! A lazy handle to the original host, not another lease or tool owner.
use crate::agent::{
    compaction_hook_process::{HookCommandError, HookProcessLauncher},
    shell_tools::workspace_host::{AuthorizeHook, AuthorizeStopHook, WorkspaceShellHost},
    stop_hook_process::StopHookLauncher,
};
use research_agent_core::{
    model::{ModelError, ModelFuture},
    submission::SubmissionRecord,
    workspace::WorkspaceIdentity,
};
use std::{path::PathBuf, sync::Arc};

/// Called only inside the admitted task's hook future. Implementations must
/// verify the exact frozen submission and use the tools' shared lazy host.
pub trait HookWorkspaceSource: Send + Sync {
    fn prepare<'a>(&'a self, record: &'a SubmissionRecord) -> ModelFuture<'a, HookWorkspace>;
    /// Blocking native permission check used by the output reader's worker.
    /// This must not acquire a lease, execute a command, or grant new access.
    fn authorize_artifact_read(
        &self,
        _record: &SubmissionRecord,
    ) -> Result<(), research_agent_core::context::compaction_hooks::HookContextError> {
        Err(research_agent_core::context::compaction_hooks::HookContextError)
    }
}

type LauncherFactory = dyn Fn(Arc<AuthorizeHook>) -> Arc<dyn HookProcessLauncher> + Send + Sync;
type StopLauncherFactory =
    dyn Fn(Arc<AuthorizeStopHook>) -> Arc<dyn StopHookLauncher> + Send + Sync;
type NativeAuthorization = dyn Fn(&WorkspaceIdentity) -> Result<(), HookCommandError> + Send + Sync;
pub struct HookWorkspace {
    pub(super) directory: PathBuf,
    launcher: Arc<LauncherFactory>,
    stop_launcher: Arc<StopLauncherFactory>,
}
impl HookWorkspace {
    /// Cloning this shell host shares its lease, writer intent and registry.
    /// Native scope authorization is checked alongside the handler's consent
    /// on the launch worker, immediately before acquiring the writer intent.
    pub(crate) async fn from_host(
        host: WorkspaceShellHost,
        native: Arc<NativeAuthorization>,
    ) -> Result<Self, ModelError> {
        tokio::task::spawn_blocking(move || {
            let directory = host
                .hook_working_directory()
                .map_err(|_| ModelError::Unavailable)?;
            let stop_host = host.clone();
            let stop_native = native.clone();
            let launcher = Arc::new(move |handler: Arc<AuthorizeHook>| {
                let native = native.clone();
                host.hook_launcher(Arc::new(move |scope, request| {
                    native(scope)?;
                    handler(scope, request)
                }))
            });
            let stop_launcher = Arc::new(move |handler: Arc<AuthorizeStopHook>| {
                let native = stop_native.clone();
                stop_host.stop_hook_launcher(Arc::new(move |scope, request| {
                    native(scope)?;
                    handler(scope, request)
                }))
            });
            Ok(Self {
                directory,
                launcher,
                stop_launcher,
            })
        })
        .await
        .map_err(|_| ModelError::Unavailable)?
    }
    pub(super) fn launcher(&self, authorize: Arc<AuthorizeHook>) -> Arc<dyn HookProcessLauncher> {
        (self.launcher)(authorize)
    }
    /// Binds handler consent to the original native workspace authorization.
    pub fn stop_launcher(&self, authorize: Arc<AuthorizeStopHook>) -> Arc<dyn StopHookLauncher> {
        (self.stop_launcher)(authorize)
    }
}
