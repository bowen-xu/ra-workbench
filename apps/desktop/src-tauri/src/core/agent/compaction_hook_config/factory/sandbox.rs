//! Use the existing Codex-derived offline Seatbelt plan builder. No new process
//! owner, profile language, network permission, or unsandboxed retry path.
use super::*;
use crate::agent::process::{OfflineSandboxPaths, prepare_offline_sandbox};

/// Native-only scope selection. Called on a blocking worker after source and
/// command admission. Grants must come from approved frozen workspace/services,
/// never from command cwd, renderer paths or the model's request content.
pub type HookSandboxPaths = dyn Fn(
        &InstructionRootIdentity<'_>,
        &ConfiguredHook,
        AgentHost,
        &SubmissionRecord,
    ) -> Result<OfflineSandboxPaths, HookConfigError>
    + Send
    + Sync;

pub(super) struct OfflineAuthority {
    inner: Arc<dyn ConfigurationAuthority>,
    paths: Arc<HookSandboxPaths>,
}
impl OfflineAuthority {
    pub(super) fn new(
        inner: Arc<dyn ConfigurationAuthority>,
        paths: Arc<HookSandboxPaths>,
    ) -> Self {
        Self { inner, paths }
    }
}
impl ConfigurationAuthority for OfflineAuthority {
    fn authorize_source(
        &self,
        root: &InstructionRootIdentity<'_>,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<(), HookConfigError> {
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
        let plan = self
            .inner
            .authorize_command(root, handler, revision, host, record)?;
        let paths = (self.paths)(root, handler, host, record)?;
        if let Some(transcript) = &plan.transcript_path {
            // The hook input must not reveal an ungranted/protected pathname.
            // Resolving is safe here on the original blocking preparation owner.
            if transcript.canonicalize().map_err(|_| HookConfigError)? != *transcript
                || !(transcript.starts_with(&paths.work)
                    || paths
                        .read_only
                        .iter()
                        .any(|read| transcript.starts_with(read)))
                || paths
                    .protected
                    .iter()
                    .any(|protected| transcript.starts_with(protected))
            {
                return Err(HookConfigError);
            }
        }
        Ok(AuthorizedHookCommand {
            process: prepare_offline_sandbox(plan.process, paths).map_err(|_| HookConfigError)?,
            transcript_path: plan.transcript_path,
            launcher: plan.launcher,
        })
    }
}
impl HookInvocationAuthority for OfflineAuthority {
    fn check(
        &self,
        id: Uuid,
        revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        // The original authority still owns current scope permissions. Do not
        // rebuild a profile or perform filesystem/SQLite work at invocation.
        self.inner.check(id, revision, request)
    }
}
