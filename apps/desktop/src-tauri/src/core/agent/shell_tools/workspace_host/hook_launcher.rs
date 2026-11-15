//! Hooks share the original lease but keep their caller-driven supervisor.
use super::*;
use crate::agent::compaction_hook_process::{
    HookCommandError, HookLaunchFuture, HookProcessLauncher,
};
use crate::agent::stop_hook_process::{StopHookLauncher, StopRequest};
use research_agent_core::context::compaction_hooks::CompactionHookRequest;
use std::sync::atomic::{AtomicBool, Ordering};

/// Blocking-worker native policy: recheck durable workspace, command consent and
/// exact model scope. No hook execution or callbacks into this lease are allowed.
pub type AuthorizeHook = dyn Fn(&WorkspaceIdentity, &CompactionHookRequest) -> Result<(), HookCommandError>
    + Send
    + Sync;
pub type AuthorizeStopHook =
    dyn Fn(&WorkspaceIdentity, &StopRequest) -> Result<(), HookCommandError> + Send + Sync;
type Authorization<R> =
    dyn Fn(&WorkspaceIdentity, &R) -> Result<(), HookCommandError> + Send + Sync;
pub(super) struct Launcher<R> {
    lease: Arc<Mutex<WorkspaceLease>>,
    scope: WorkspaceIdentity,
    protected: Vec<PathBuf>,
    read_only: Vec<PathBuf>,
    authorize: Arc<Authorization<R>>,
}
impl<R: Clone + Send + Sync + 'static> Launcher<R> {
    pub(super) fn new(
        lease: Arc<Mutex<WorkspaceLease>>,
        scope: WorkspaceIdentity,
        protected: Vec<PathBuf>,
        read_only: Vec<PathBuf>,
        authorize: Arc<Authorization<R>>,
    ) -> Self {
        Self {
            lease,
            scope,
            protected,
            read_only,
            authorize,
        }
    }
    fn launch_owned<'a>(
        &'a self,
        request: &'a R,
        process: ProcessSpec,
        limits: ProcessLimits,
    ) -> HookLaunchFuture<'a> {
        Box::pin(async move {
            let active = Arc::new(AtomicBool::new(true));
            let _admission = Admission(active.clone());
            let lease = self.lease.clone();
            let scope = self.scope.clone();
            let authorize = self.authorize.clone();
            let request = request.clone();
            let protected = self.protected.clone();
            let read_only = self.read_only.clone();
            tokio::task::spawn_blocking(move || {
                if !active.load(Ordering::Acquire) {
                    return Err(HookCommandError);
                }
                authorize(&scope, &request)?;
                let mut lease = lease.lock().map_err(|_| HookCommandError)?;
                if !active.load(Ordering::Acquire) {
                    return Err(HookCommandError);
                }
                let work = lease.working_directory().map_err(|_| HookCommandError)?;
                let process = prepare_offline_sandbox(
                    process,
                    OfflineSandboxPaths {
                        work,
                        read_only,
                        protected,
                        allow_system_metadata: false,
                    },
                )
                .map_err(|_| HookCommandError)?;
                if !active.load(Ordering::Acquire) {
                    return Err(HookCommandError);
                }
                lease
                    .begin_execution()
                    .map_err(|_| HookCommandError)?
                    .spawn_hook(process, limits)
                    .map_err(|_| HookCommandError)
            })
            .await
            .map_err(|_| HookCommandError)?
        })
    }
}

struct Admission(Arc<AtomicBool>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl HookProcessLauncher for Launcher<CompactionHookRequest> {
    fn launch<'a>(
        &'a self,
        request: &'a CompactionHookRequest,
        process: ProcessSpec,
        limits: ProcessLimits,
    ) -> HookLaunchFuture<'a> {
        if request.conversation_id != self.scope.conversation_id
            || request.turn_id != self.scope.turn_id
            || request.operation_id.is_nil()
        {
            return Box::pin(async { Err(HookCommandError) });
        }
        self.launch_owned(request, process, limits)
    }
}
impl StopHookLauncher for Launcher<StopRequest> {
    fn launch<'a>(
        &'a self,
        request: &'a StopRequest,
        process: ProcessSpec,
        limits: ProcessLimits,
    ) -> HookLaunchFuture<'a> {
        if request.conversation_id != self.scope.conversation_id
            || request.turn_id != self.scope.turn_id
        {
            return Box::pin(async { Err(HookCommandError) });
        }
        self.launch_owned(request, process, limits)
    }
}
