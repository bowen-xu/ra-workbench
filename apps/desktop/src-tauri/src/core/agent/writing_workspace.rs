//! Captured native workspace input and closed manuscript repository operations.
//! These are internal adapters, not a plugin API exposing arbitrary SQL access.
use super::{
    workspace::{WorkspaceInput, WorkspaceLease, WorkspaceStore},
    writing_context::WritingContextError,
};
use research_agent_core::workspace::{
    EditorVersion, FileVersion, WorkspaceIdentity, WorkspaceManifest,
};
use std::path::Path;
use uuid::Uuid;

/// Supplied by native editor-session coordination, never model arguments.
pub struct EditorBuffer {
    pub path: String,
    pub expected_disk_sha256: String,
    pub editor: EditorVersion,
    pub text: String,
}
pub struct WritingWorkspaceRequest {
    pub manuscript_id: Uuid,
    pub buffers: Vec<EditorBuffer>,
}
pub struct WritingWorkspaceSnapshot {
    pub manifest: WorkspaceManifest,
    pub(crate) inputs: Vec<WorkspaceInput>,
}
impl WritingWorkspaceSnapshot {
    /// Store must already be independently authorized as app-owned storage.
    pub fn create(self, store: &WorkspaceStore) -> Result<WorkspaceLease, WritingContextError> {
        store
            .create(self.manifest, self.inputs)
            .map_err(|_| WritingContextError::Unavailable)
    }
}

type Snapshot = fn(
    &rusqlite::Connection,
    &Path,
    WorkspaceIdentity,
    WritingWorkspaceRequest,
) -> Result<WritingWorkspaceSnapshot, WritingContextError>;
type CheckUnregistered = fn(&rusqlite::Connection, Uuid, &str) -> Result<(), WritingContextError>;
type CheckRegistered =
    fn(&rusqlite::Connection, Uuid, &str, &FileVersion) -> Result<(), WritingContextError>;
type UpdateRegistered = fn(
    &rusqlite::Connection,
    Uuid,
    &str,
    &FileVersion,
    Option<&FileVersion>,
) -> Result<(), WritingContextError>;
pub(crate) type ReadCheckpoint = fn(
    &rusqlite::Connection,
    Uuid,
    &str,
) -> Result<
    Option<crate::agent::writing_checkpoint_contracts::CheckpointRecord>,
    WritingContextError,
>;

#[derive(Clone, Copy)]
pub(crate) struct WritingWorkspaceSource {
    pub snapshot: Snapshot,
    pub check_unregistered: CheckUnregistered,
    pub check_registered: CheckRegistered,
    pub register_created: CheckRegistered,
    pub update_registered: UpdateRegistered,
    pub read_checkpoint: ReadCheckpoint,
}

#[path = "writing_workspace/snapshot.rs"]
pub(crate) mod snapshot;

#[path = "writing_workspace/absence.rs"]
pub(crate) mod absence;

#[path = "writing_workspace/index.rs"]
pub(crate) mod index;
