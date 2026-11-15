//! Native non-Git working copies. Call on a blocking worker. The host supplies
//! authorized frozen bytes, never a model-selected source or storage root.
use research_agent_core::workspace::WorkspaceError;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LeaseError {
    #[error("invalid workspace data")]
    Invalid,
    #[error("workspace source or identity changed")]
    Conflict,
    #[error("target file content no longer matches the patch context")]
    ContentMismatch,
    #[error("patch context matches multiple locations")]
    AmbiguousPatch,
    #[error("workspace exceeds its limits")]
    Budget,
    #[error("workspace preparation is incomplete")]
    Incomplete,
    #[error("workspace is already owned by another operation")]
    Busy,
    #[error("workspace preparation failed and cleanup requires retry")]
    CleanupRequired,
    #[error("workspace access is unauthorized")]
    Unauthorized,
    #[error("workspace storage is unavailable")]
    Unavailable,
}
impl From<WorkspaceError> for LeaseError {
    fn from(error: WorkspaceError) -> Self {
        match error {
            WorkspaceError::Invalid => Self::Invalid,
            WorkspaceError::Conflict => Self::Conflict,
            WorkspaceError::Budget => Self::Budget,
        }
    }
}
impl From<std::io::Error> for LeaseError {
    fn from(_: std::io::Error) -> Self {
        Self::Unavailable
    }
}
impl From<crate::agent::instructions::InstructionReadError> for LeaseError {
    fn from(error: crate::agent::instructions::InstructionReadError) -> Self {
        use crate::agent::instructions::InstructionReadError;
        match error {
            InstructionReadError::Budget => Self::Budget,
            InstructionReadError::InvalidScope => Self::Invalid,
            _ => Self::Unavailable,
        }
    }
}

#[cfg(unix)]
#[path = "workspace/discard.rs"]
mod discard;
#[cfg(unix)]
pub use discard::DiscardState;
#[cfg(unix)]
#[path = "workspace/lease.rs"]
mod lease;
#[cfg(unix)]
#[path = "workspace/storage.rs"]
pub(crate) mod storage;
#[cfg(unix)]
#[path = "workspace/tree.rs"]
mod tree;
pub(crate) use tree::version as file_version;
#[path = "workspace/sandbox.rs"]
pub(crate) mod sandbox;
#[cfg(unix)]
pub use lease::{
    ChangeContent, ChangeSide, ReviewContent, StorageIdentity, TextPatchOperation, WorkspaceInput,
    WorkspaceLease, WorkspaceStore,
};
// Other platforms have no filesystem backend yet; no unisolated fallback.

#[cfg(unix)]
#[path = "workspace/execution.rs"]
mod execution;
#[cfg(unix)]
pub use execution::WorkspaceExecution;

#[cfg(unix)]
#[path = "workspace/object_staging.rs"]
pub(crate) mod object_staging;
#[cfg(unix)]
#[path = "workspace/objects.rs"]
mod objects;
#[cfg(unix)]
pub use objects::ProposalObjects;

#[cfg(unix)]
#[path = "workspace/disk.rs"]
mod disk;
#[cfg(unix)]
pub use disk::{DiskError, DiskTarget};
