//! Borrow the writing turn's existing lazy native owner, never another lease.
use crate::agent::{
    repository::execution_workspaces::Binding, shell_tools::workspace_host::WorkspaceShellHost,
};
use research_agent_core::{runner::RunFuture, submission::SubmissionRecord};
use std::sync::Arc;

pub(crate) struct FileWorkspace {
    pub(crate) host: WorkspaceShellHost,
    pub(crate) binding: Binding,
    pub(crate) grant: super::file::WorkspaceGrant,
}

pub(crate) trait FileWorkspaceSource: Send + Sync {
    fn workspace<'a>(&'a self, record: &'a SubmissionRecord) -> RunFuture<'a, FileWorkspace>;
    /// Observe the current owner without causing a workspace transition.
    fn current_lease(
        &self,
        _record: &SubmissionRecord,
    ) -> Result<Option<uuid::Uuid>, research_agent_core::runner::RunError> {
        Ok(None)
    }
}

pub(crate) type FileWorkspaceHandle = Option<Arc<dyn FileWorkspaceSource>>;

pub(crate) fn original_grant() -> super::file::WorkspaceGrant {
    super::file::WorkspaceGrant::Original
}

pub(crate) fn generation_grant(
    generation: u32,
    predecessor: uuid::Uuid,
) -> super::file::WorkspaceGrant {
    super::file::WorkspaceGrant::Generation {
        generation,
        predecessor,
    }
}
