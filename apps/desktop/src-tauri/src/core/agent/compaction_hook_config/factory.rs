//! Fixed user config source composed with host-owned trust and live authority.
use super::*;
use crate::agent::{
    compaction_hook_factory::{CompactionHookFactory, PreparedCompactionHooks},
    instructions::InstructionRootIdentity,
    model::AgentHost,
};
use research_agent_core::{
    model::{ModelError, ModelFuture},
    submission::SubmissionRecord,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// Dropping the task-owned future retires preparation. Blocking callbacks already
/// past the ownership check cannot be interrupted. Later checks refuse work.
struct PreparationOwner(Arc<AtomicBool>);
impl Drop for PreparationOwner {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Native policy owns source access, version consent and command sandbox plans.
/// No method may execute a command. Approval must refer to this exact submission.
pub trait ConfigurationAuthority: HookInvocationAuthority {
    fn authorize_source(
        &self,
        root: &InstructionRootIdentity<'_>,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<(), HookConfigError>;
    fn authorize_command(
        &self,
        root: &InstructionRootIdentity<'_>,
        handler: &ConfiguredHook,
        revision: &str,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<AuthorizedHookCommand, HookConfigError>;
}

/// Explicitly inject into AgentModelResolver. No app startup discovery or consent.
pub struct ConfiguredHookFactory {
    roots: Vec<PathBuf>,
    authority: Arc<dyn ConfigurationAuthority>,
    observer: Arc<dyn HookHandlerObserver>,
}
impl ConfiguredHookFactory {
    /// MacOS native host supplies scoped workspace/read/protected grants on the
    /// blocking preparation worker. Every resulting plan uses the OS sandbox.
    #[cfg(target_os = "macos")]
    pub fn with_offline_sandbox(mut self, paths: Arc<sandbox::HookSandboxPaths>) -> Self {
        self.authority = Arc::new(sandbox::OfflineAuthority::new(self.authority, paths));
        self
    }
    /// Native composition supplies one shared consent controller; this does not
    /// approve commands or replace the host's source access and sandbox policy.
    pub fn with_consent(
        self,
        consent: Arc<consent::SqliteHookConsent>,
    ) -> consented::ConsentedHookFactory {
        consented::ConsentedHookFactory::new(self, consent)
    }
    pub fn new(
        root: PathBuf,
        authority: Arc<dyn ConfigurationAuthority>,
        observer: Arc<dyn HookHandlerObserver>,
    ) -> Result<Self, HookConfigError> {
        Self::from_roots(vec![root], authority, observer)
    }

    /// Host-selected sources in deterministic order. Each root and revision is
    /// independently authorized; duplicate handler identities never shadow.
    pub fn from_roots(
        roots: Vec<PathBuf>,
        authority: Arc<dyn ConfigurationAuthority>,
        observer: Arc<dyn HookHandlerObserver>,
    ) -> Result<Self, HookConfigError> {
        if !(1..=32).contains(&roots.len())
            || roots.iter().any(|root| !root.is_absolute())
            || roots.iter().collect::<std::collections::HashSet<_>>().len() != roots.len()
        {
            return Err(HookConfigError);
        }
        Ok(Self {
            roots,
            authority,
            observer,
        })
    }
}
struct LiveAuthority {
    policy: Arc<dyn ConfigurationAuthority>,
    conversation_id: Uuid,
    turn_id: Uuid,
}
impl HookInvocationAuthority for LiveAuthority {
    fn check(
        &self,
        id: Uuid,
        revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        if request.conversation_id != self.conversation_id || request.turn_id != self.turn_id {
            return Err(HookConfigError);
        }
        self.policy.check(id, revision, request)
    }
}
impl CompactionHookFactory for ConfiguredHookFactory {
    fn prepare<'a>(
        &'a self,
        host: AgentHost,
        record: &'a SubmissionRecord,
    ) -> ModelFuture<'a, PreparedCompactionHooks> {
        let roots = self.roots.clone();
        let authority = self.authority.clone();
        let observer = self.observer.clone();
        let record = record.clone();
        Box::pin(async move {
            let active = Arc::new(AtomicBool::new(true));
            let _owner = PreparationOwner(active.clone());
            // Blocking I/O already entered may finish after cancellation. Check
            // ownership around policy callbacks before continuing preparation.
            tokio::task::spawn_blocking(move || {
                let live = Arc::new(LiveAuthority {
                    policy: authority.clone(),
                    conversation_id: record.conversation_id,
                    turn_id: record.turn_id,
                });
                let mut configurations = Vec::new();
                for root in roots {
                    if !active.load(Ordering::Acquire) {
                        return Err(ModelError::Canceled);
                    }
                    let mut source_identity = None;
                    let Some(configuration) = source::load(&root, |identity| {
                        if !active.load(Ordering::Acquire) {
                            return Err(HookConfigError);
                        }
                        authority.authorize_source(identity, host, &record)?;
                        source_identity = Some((
                            identity.canonical_path.to_path_buf(),
                            identity.device,
                            identity.inode,
                        ));
                        active
                            .load(Ordering::Acquire)
                            .then_some(())
                            .ok_or(HookConfigError)
                    })
                    .map_err(|_| ModelError::Unavailable)?
                    else {
                        continue;
                    };
                    configurations.push((
                        source_identity.ok_or(ModelError::Unavailable)?,
                        configuration,
                    ));
                }
                if configurations.is_empty() {
                    return Ok(None);
                }
                let mut ids = std::collections::HashSet::new();
                for handler in configurations
                    .iter()
                    .flat_map(|(_, configuration)| configuration.handlers())
                    .filter(|handler| handler.enabled)
                {
                    if !ids.insert(handler.id) || ids.len() > 32 {
                        return Err(ModelError::Unavailable);
                    }
                }
                let mut handlers = Vec::new();
                for ((path, device, inode), configuration) in configurations {
                    let root = InstructionRootIdentity {
                        canonical_path: &path,
                        device,
                        inode,
                    };
                    handlers.extend(
                        configuration
                            .authorized_handlers(
                                record.input.model.clone(),
                                |handler, revision| {
                                    if !active.load(Ordering::Acquire) {
                                        return Err(HookConfigError);
                                    }
                                    let plan = authority.authorize_command(
                                        &root, handler, revision, host, &record,
                                    )?;
                                    if !active.load(Ordering::Acquire) {
                                        return Err(HookConfigError);
                                    }
                                    Ok(plan)
                                },
                                live.clone(),
                            )
                            .map_err(|_| ModelError::Unavailable)?,
                    );
                }
                let hooks = ConcurrentCompactionHooks::new(handlers, observer)
                    .map_err(|_| ModelError::Unavailable)?;
                Ok(Some(Arc::new(hooks) as Arc<dyn CompactionHooks>))
            })
            .await
            .map_err(|_| ModelError::Unavailable)?
        })
    }
}

#[path = "factory/consented.rs"]
pub mod consented;
#[cfg(target_os = "macos")]
#[path = "factory/sandbox.rs"]
pub mod sandbox;
#[cfg(test)]
#[path = "factory/tests.rs"]
mod tests;
