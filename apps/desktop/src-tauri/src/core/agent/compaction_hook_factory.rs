//! Trusted native preparation port shared by regular and manual compaction hosts.
use super::model::AgentHost;
use research_agent_core::{
    context::compaction_hooks::CompactionHooks, model::ModelFuture, submission::SubmissionRecord,
};
use std::sync::Arc;

pub type PreparedCompactionHooks = Option<Arc<dyn CompactionHooks>>;
#[cfg(target_os = "macos")]
pub type HookWorkspaceHandle =
    Option<Arc<dyn super::compaction_hook_config::native::HookWorkspaceSource>>;
#[cfg(not(target_os = "macos"))]
pub type HookWorkspaceHandle = Option<()>;

/// Resolve configuration and authorize commands for this immutable submission.
/// Implementations own source/trust checks and must freeze exact model identity.
/// No command may launch while preparing. All I/O is task-owned; no detached work.
/// This port is never deserialized from IPC, model output or provider capabilities.
pub trait CompactionHookFactory: Send + Sync {
    fn prepare<'a>(
        &'a self,
        host: AgentHost,
        record: &'a SubmissionRecord,
    ) -> ModelFuture<'a, PreparedCompactionHooks>;
}
