//! macOS production execution boundary: frozen lease, offline Seatbelt, bounded
//! process ownership. Conversation admission supplies the authorization callback;
//! this module does not infer grants from model text or instruction files.
use super::*;
use crate::agent::{
    process::{
        OfflineSandboxPaths, ProcessError, ProcessLimits, ProcessSpec, TerminalSize,
        prepare_offline_sandbox,
    },
    workspace::WorkspaceLease,
};
use crate::data_interface::IntoDatabase;
use research_agent_core::workspace::FileChange;
use std::{collections::BTreeMap, path::PathBuf};

#[path = "workspace_host/hook_launcher.rs"]
mod hook_launcher;
pub use hook_launcher::{AuthorizeHook, AuthorizeStopHook};

/// Called on the blocking pool, including immediately before process admission.
/// Must check current task authorization; must not call back into this host or
/// registry. Scope and protected paths are supplied by trusted application code.
pub type AuthorizeShell =
    dyn Fn(&WorkspaceIdentity, &ShellRequest) -> Result<(), RunError> + Send + Sync;
#[derive(Clone)]
pub struct WorkspaceShellHost {
    lease: Arc<Mutex<WorkspaceLease>>,
    registry: Arc<ProcessRegistry>,
    scope: WorkspaceIdentity,
    protected: Vec<PathBuf>,
    read_only: Vec<PathBuf>,
    authorize: Arc<AuthorizeShell>,
}
impl WorkspaceShellHost {
    /// Blocking worker only; validates the original opened work directory.
    pub(crate) fn hook_working_directory(&self) -> Result<PathBuf, RunError> {
        self.lease
            .lock()
            .map_err(|_| RunError::ToolRejected)?
            .working_directory()
            .map_err(|_| RunError::ToolRejected)
    }
    /// Native hook policy must authorize the exact invocation separately from
    /// shell permission. The launcher shares this host's existing lease.
    pub fn hook_launcher(
        &self,
        authorize: Arc<AuthorizeHook>,
    ) -> Arc<dyn crate::agent::compaction_hook_process::HookProcessLauncher> {
        Arc::new(hook_launcher::Launcher::new(
            self.lease.clone(),
            self.scope.clone(),
            self.protected.clone(),
            self.read_only.clone(),
            authorize,
        ))
    }
    /// Stop hooks share the same sandbox and writer lease; this callback must
    /// independently revalidate the exact handler, configuration and grant.
    pub fn stop_hook_launcher(
        &self,
        authorize: Arc<AuthorizeStopHook>,
    ) -> Arc<dyn crate::agent::stop_hook_process::StopHookLauncher> {
        Arc::new(hook_launcher::Launcher::new(
            self.lease.clone(),
            self.scope.clone(),
            self.protected.clone(),
            self.read_only.clone(),
            authorize,
        ))
    }
    /// Bind the real SQLite authorization store to this exact lease. Call on a
    /// blocking worker, without holding the connection mutex. Construction does
    /// not grant permission; every operation and launch rechecks durable consent.
    pub(crate) fn from_approved_binding(
        lease: WorkspaceLease,
        registry: Arc<ProcessRegistry>,
        protected: Vec<PathBuf>,
        connection: impl IntoDatabase,
        binding: crate::agent::repository::execution_workspaces::Binding,
    ) -> Result<Self, RunError> {
        use crate::agent::repository::execution_workspaces;
        let connection = connection.into_database();
        let manifest = lease.manifest().clone();
        {
            let db = connection.lock().map_err(|_| RunError::EventSink)?;
            execution_workspaces::authorize(&db, &binding, &manifest)
                .map_err(|_| RunError::ToolRejected)?;
        }
        let authorize: Arc<AuthorizeShell> = Arc::new(move |scope, _request| {
            if scope != &manifest.identity {
                return Err(RunError::ToolRejected);
            }
            let db = connection.lock().map_err(|_| RunError::EventSink)?;
            execution_workspaces::authorize(&db, &binding, &manifest).map_err(|error| match error {
                execution_workspaces::Error::Storage => RunError::EventSink,
                _ => RunError::ToolRejected,
            })
        });
        Self::new(lease, registry, protected, authorize)
    }
    /// A successor keeps its own durable grant and rechecks its predecessor at
    /// every shell admission. It never inherits the revoked original grant.
    pub(crate) fn from_approved_generation(
        lease: WorkspaceLease,
        registry: Arc<ProcessRegistry>,
        protected: Vec<PathBuf>,
        connection: impl IntoDatabase,
        binding: crate::agent::repository::execution_workspaces::Binding,
        generation: u32,
        predecessor: Uuid,
    ) -> Result<Self, RunError> {
        use crate::agent::repository::execution_workspaces;
        let connection = connection.into_database();
        let manifest = lease.manifest().clone();
        {
            let db = connection.lock().map_err(|_| RunError::EventSink)?;
            execution_workspaces::generation::authorize(
                &db,
                &binding,
                generation,
                predecessor,
                &manifest,
            )
            .map_err(|_| RunError::ToolRejected)?;
        }
        let authorize: Arc<AuthorizeShell> = Arc::new(move |scope, _request| {
            if scope != &manifest.identity {
                return Err(RunError::ToolRejected);
            }
            let db = connection.lock().map_err(|_| RunError::EventSink)?;
            execution_workspaces::generation::authorize(
                &db,
                &binding,
                generation,
                predecessor,
                &manifest,
            )
            .map_err(|error| match error {
                execution_workspaces::Error::Storage => RunError::EventSink,
                _ => RunError::ToolRejected,
            })
        });
        Self::new(lease, registry, protected, authorize)
    }
    /// Blocking-worker only. The lease must contain already authorized frozen
    /// source bytes. Protected roots must include original research/auth/storage
    /// resources; working-copy control paths are added here automatically.
    pub fn new(
        lease: WorkspaceLease,
        registry: Arc<ProcessRegistry>,
        mut protected: Vec<PathBuf>,
        authorize: Arc<AuthorizeShell>,
    ) -> Result<Self, RunError> {
        if protected.is_empty() {
            return Err(RunError::ToolRejected);
        }
        let work = lease
            .working_directory()
            .map_err(|_| RunError::ToolRejected)?;
        let parent = work.parent().ok_or(RunError::ToolRejected)?;
        for name in [
            "base",
            "manifest.json",
            "owner.lock",
            "execution.json",
            "ready",
        ] {
            protected.push(parent.join(name));
        }
        // Validate static policy before recording a persistent execution intent.
        prepare_offline_sandbox(
            Self::spec(work.clone(), "true".into()),
            OfflineSandboxPaths {
                work,
                read_only: vec![],
                protected: protected.clone(),
                allow_system_metadata: false,
            },
        )
        .map_err(|_| RunError::ToolRejected)?;
        Ok(Self {
            scope: lease.manifest().identity.clone(),
            lease: Arc::new(Mutex::new(lease)),
            registry,
            protected,
            read_only: vec![],
            authorize,
        })
    }
    pub(crate) fn with_read_only(mut self, paths: Vec<PathBuf>) -> Result<Self, RunError> {
        let work = self
            .lease
            .lock()
            .map_err(|_| RunError::EventSink)?
            .working_directory()
            .map_err(|_| RunError::ToolRejected)?;
        prepare_offline_sandbox(
            Self::spec(work.clone(), "true".into()),
            OfflineSandboxPaths {
                work,
                read_only: paths.clone(),
                protected: self.protected.clone(),
                allow_system_metadata: false,
            },
        )
        .map_err(|_| RunError::ToolRejected)?;
        self.read_only = paths;
        Ok(self)
    }
    fn spec(work: PathBuf, command: String) -> ProcessSpec {
        ProcessSpec {
            program: "/bin/sh".into(),
            arguments: vec!["-c".into(), command.into()],
            directory: work,
            environment: BTreeMap::from([
                ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
                ("TERM".into(), "xterm-256color".into()),
            ]),
        }
    }
    /// Inspection only; still requires a separate revision-checked ChangeSet
    /// acceptance to update original sources. Active/uncertain writers fail closed.
    pub fn changes(&self) -> RunFuture<'_, Vec<FileChange>> {
        self.inspect_owned(|lease| lease.changes().map_err(|_| RunError::EventSink))
    }

    /// Trusted native readers share the owned lease; they cannot start a process
    /// or acquire another owner. Each reader must validate its durable scope and
    /// use the lease's version-checked, quiescent read operations.
    pub(crate) fn inspect_owned<T: Send + 'static>(
        &self,
        read: impl FnOnce(&WorkspaceLease) -> Result<T, RunError> + Send + 'static,
    ) -> RunFuture<'_, T> {
        let lease = self.lease.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let lease = lease.lock().map_err(|_| RunError::EventSink)?;
                read(&lease)
            })
            .await
            .map_err(|_| RunError::EventSink)?
        })
    }

    /// A trusted native file adapter uses this same owner for its entire
    /// quiescence/authorization/CAS/journal operation. The blocking worker keeps
    /// the lease alive even if its asynchronous waiter is dropped.
    pub(crate) fn with_owned_lease_mut<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut WorkspaceLease) -> T + Send + 'static,
    ) -> RunFuture<'_, T> {
        let lease = self.lease.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut lease = lease.lock().map_err(|_| RunError::EventSink)?;
                Ok(operation(&mut lease))
            })
            .await
            .map_err(|_| RunError::EventSink)?
        })
    }
}
impl ShellHost for WorkspaceShellHost {
    fn authorize(&self, scope: WorkspaceIdentity, request: ShellRequest) -> RunFuture<'_, ()> {
        let authorize = self.authorize.clone();
        let expected = self.scope.clone();
        Box::pin(async move {
            if scope != expected {
                return Err(RunError::ToolRejected);
            }
            tokio::task::spawn_blocking(move || authorize(&scope, &request))
                .await
                .map_err(|_| RunError::EventSink)?
        })
    }
    fn launch(
        &self,
        registry: Arc<ProcessRegistry>,
        scope: WorkspaceIdentity,
        request: ExecRequest,
    ) -> RunFuture<'_, Uuid> {
        let lease = self.lease.clone();
        let authorize = self.authorize.clone();
        let protected = self.protected.clone();
        let read_only = self.read_only.clone();
        let valid_scope = self.scope == scope && Arc::ptr_eq(&registry, &self.registry);
        Box::pin(async move {
            if !valid_scope {
                return Err(RunError::ToolRejected);
            }
            tokio::task::spawn_blocking(move || {
                authorize(&scope, &ShellRequest::Exec(request.clone()))?;
                let mut lease = lease.lock().map_err(|_| RunError::EventSink)?;
                let work = lease.working_directory().map_err(|_| RunError::EventSink)?;
                let spec = prepare_offline_sandbox(
                    Self::spec(work.clone(), request.command),
                    OfflineSandboxPaths {
                        work,
                        read_only,
                        protected,
                        allow_system_metadata: false,
                    },
                )
                .map_err(|_| RunError::ToolRejected)?;
                registry
                    .start(scope, || {
                        let execution = lease
                            .begin_execution()
                            .map_err(|_| ProcessError::ResourceRelease)?;
                        execution.spawn(
                            spec,
                            ProcessLimits {
                                // Worst-case JSON escaping remains below the 256 KiB
                                // shell envelope budget; caller coordinates runner limits.
                                retained_bytes: 16 * 1024,
                                execution_timeout: Duration::from_secs(300),
                                drain_timeout: Duration::from_secs(5),
                            },
                            request.terminal.then_some(TerminalSize {
                                rows: 24,
                                columns: 80,
                            }),
                        )
                    })
                    .map_err(|_| RunError::EventSink)
            })
            .await
            .map_err(|_| RunError::EventSink)?
        })
    }
}

#[cfg(test)]
#[path = "workspace_host/tests.rs"]
mod tests;
