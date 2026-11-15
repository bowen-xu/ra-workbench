//! Durable consent gates the existing factory without replacing its host policy
//! or shared process/task owner. Bindings live only as long as the prepared owner.
use super::*;
use crate::agent::{
    compaction_hook_config::consent::{LiveHookGrant, SqliteHookConsent},
    repository::compaction_hook_trust::GrantKey,
};
use std::sync::RwLock;

pub struct ConsentedHookFactory {
    configured: ConfiguredHookFactory,
    consent: Arc<SqliteHookConsent>,
}
impl ConsentedHookFactory {
    pub(super) fn new(configured: ConfiguredHookFactory, consent: Arc<SqliteHookConsent>) -> Self {
        Self {
            configured,
            consent,
        }
    }
}
impl CompactionHookFactory for ConsentedHookFactory {
    fn prepare<'a>(
        &'a self,
        host: AgentHost,
        record: &'a SubmissionRecord,
    ) -> ModelFuture<'a, PreparedCompactionHooks> {
        Box::pin(async move {
            let authority = Arc::new(ScopedAuthority {
                inner: self.configured.authority.clone(),
                consent: self.consent.clone(),
                host,
                record: record.clone(),
                bindings: RwLock::new(Vec::new()),
            });
            let factory = ConfiguredHookFactory::from_roots(
                self.configured.roots.clone(),
                authority,
                self.configured.observer.clone(),
            )
            .map_err(|_| ModelError::Unavailable)?;
            factory.prepare(host, record).await
        })
    }
}

struct Binding {
    id: Uuid,
    revision: String,
    path: PathBuf,
    device: u64,
    inode: u64,
    key: GrantKey,
    grant: Arc<LiveHookGrant>,
}
struct ScopedAuthority {
    inner: Arc<dyn ConfigurationAuthority>,
    consent: Arc<SqliteHookConsent>,
    host: AgentHost,
    record: SubmissionRecord,
    bindings: RwLock<Vec<Binding>>,
}
impl ScopedAuthority {
    fn matches(&self, host: AgentHost, record: &SubmissionRecord) -> bool {
        std::mem::discriminant(&host) == std::mem::discriminant(&self.host)
            && record == &self.record
    }
}
impl ConfigurationAuthority for ScopedAuthority {
    fn authorize_source(
        &self,
        root: &InstructionRootIdentity<'_>,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<(), HookConfigError> {
        if !self.matches(host, record) {
            return Err(HookConfigError);
        }
        self.inner.authorize_source(root, host, record)
    }
    fn authorize_command(
        &self,
        root: &InstructionRootIdentity<'_>,
        handler: &ConfiguredHook,
        revision: &str,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<AuthorizedHookCommand, HookConfigError> {
        if !self.matches(host, record) {
            return Err(HookConfigError);
        }
        let key = GrantKey::new(
            root.canonical_path,
            handler.id,
            host,
            record.input.model.clone(),
        )
        .map_err(|_| HookConfigError)?;
        let grant = self.consent.load(&key).map_err(|_| HookConfigError)?;
        if !grant.allows(&key, root, revision) {
            return Err(HookConfigError);
        }
        // Consent is necessary, never sufficient: the native host still supplies
        // a permitted/sandboxed plan and may refuse an otherwise approved hook.
        let plan = self
            .inner
            .authorize_command(root, handler, revision, host, record)?;
        let mut bindings = self.bindings.write().map_err(|_| HookConfigError)?;
        if bindings.len() >= 32 || bindings.iter().any(|binding| binding.id == handler.id) {
            return Err(HookConfigError);
        }
        bindings.push(Binding {
            id: handler.id,
            revision: revision.into(),
            path: root.canonical_path.into(),
            device: root.device,
            inode: root.inode,
            key,
            grant,
        });
        Ok(plan)
    }
}
impl HookInvocationAuthority for ScopedAuthority {
    fn check(
        &self,
        id: Uuid,
        revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        if request.conversation_id != self.record.conversation_id
            || request.turn_id != self.record.turn_id
            || request.model != self.record.input.model
        {
            return Err(HookConfigError);
        }
        let bindings = self.bindings.try_read().map_err(|_| HookConfigError)?;
        let binding = bindings
            .iter()
            .find(|binding| binding.id == id && binding.revision == revision)
            .ok_or(HookConfigError)?;
        let root = InstructionRootIdentity {
            canonical_path: &binding.path,
            device: binding.device,
            inode: binding.inode,
        };
        if !binding.grant.allows(&binding.key, &root, revision) {
            return Err(HookConfigError);
        }
        drop(bindings);
        self.inner.check(id, revision, request)
    }
}
