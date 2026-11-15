//! Borrowed synchronous manuscript adapters. No lock, connection or worker owner
//! is created here; the caller retains authorization and its existing transaction.
use super::AppStore;
use crate::agent::{
    writing_context::WritingContextError as Error,
    writing_workspace::{WritingWorkspaceRequest, WritingWorkspaceSnapshot},
};
#[cfg(test)]
use research_agent_core::workspace::FileVersion;
use research_agent_core::workspace::WorkspaceIdentity;
use rusqlite::Connection;
use std::path::Path;
use uuid::Uuid;

impl AppStore {
    pub(crate) fn writing_workspace_source(
        &self,
    ) -> Option<crate::agent::writing_workspace::WritingWorkspaceSource> {
        self.writing_workspace_source
    }

    pub(crate) fn writing_checkpoint_source(
        &self,
    ) -> Option<crate::agent::writing_workspace::ReadCheckpoint> {
        self.writing_workspace_source
            .map(|source| source.read_checkpoint)
    }

    pub(crate) fn snapshot_writing_workspace_in(
        &self,
        db: &Connection,
        root: &Path,
        scope: WorkspaceIdentity,
        request: WritingWorkspaceRequest,
        authorize: impl FnOnce(&WorkspaceIdentity, Uuid) -> Result<(), Error>,
    ) -> Result<WritingWorkspaceSnapshot, Error> {
        // Match the original snapshot entry: authorization precedes any read.
        authorize(&scope, request.manuscript_id)?;
        let source = self.writing_workspace_source.ok_or(Error::Unavailable)?;
        (source.snapshot)(db, root, scope, request)
    }

    pub(crate) fn check_absent_writing_file(
        &self,
        root: &Path,
        manuscript: Uuid,
        path: &str,
    ) -> Result<(), Error> {
        crate::agent::writing_workspace::absence::check_absent(root, manuscript, path)
    }

    #[cfg(test)]
    pub(crate) fn check_unregistered_writing_file_in(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
    ) -> Result<(), Error> {
        let source = self.writing_workspace_source.ok_or(Error::Unavailable)?;
        (source.check_unregistered)(db, manuscript, path)
    }

    #[cfg(test)]
    pub(crate) fn check_registered_writing_file_in(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        before: &FileVersion,
    ) -> Result<(), Error> {
        let source = self.writing_workspace_source.ok_or(Error::Unavailable)?;
        (source.check_registered)(db, manuscript, path, before)
    }

    #[cfg(test)]
    pub(crate) fn register_created_writing_file_in(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        version: &FileVersion,
    ) -> Result<(), Error> {
        let source = self.writing_workspace_source.ok_or(Error::Unavailable)?;
        (source.register_created)(db, manuscript, path, version)
    }

    #[cfg(test)]
    pub(crate) fn update_registered_writing_file_in(
        &self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
        before: &FileVersion,
        after: Option<&FileVersion>,
    ) -> Result<(), Error> {
        let source = self.writing_workspace_source.ok_or(Error::Unavailable)?;
        (source.update_registered)(db, manuscript, path, before, after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_workspace_source_retains_authorization_order_and_has_no_fallback() {
        let root = tempfile::tempdir().unwrap();
        let mut store = AppStore::for_test(
            Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        );
        store.writing_workspace_source = None;
        let db = store.connection().unwrap();
        let scope = || WorkspaceIdentity {
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            lease_id: Uuid::new_v4(),
            environment_id: "manuscript:missing".into(),
        };
        let request = || WritingWorkspaceRequest {
            manuscript_id: Uuid::new_v4(),
            buffers: vec![],
        };
        assert!(matches!(
            store.snapshot_writing_workspace_in(&db, root.path(), scope(), request(), |_, _| Err(
                Error::InvalidInput
            )),
            Err(Error::InvalidInput)
        ));
        assert!(matches!(
            store.snapshot_writing_workspace_in(
                &db,
                root.path(),
                scope(),
                request(),
                |_, _| Ok(())
            ),
            Err(Error::Unavailable)
        ));
        let version = FileVersion {
            sha256: "0".repeat(64),
            byte_size: 0,
            executable: false,
        };
        let id = Uuid::new_v4();
        for result in [
            store.check_unregistered_writing_file_in(&db, id, "main.tex"),
            store.check_registered_writing_file_in(&db, id, "main.tex", &version),
            store.register_created_writing_file_in(&db, id, "main.tex", &version),
            store.update_registered_writing_file_in(&db, id, "main.tex", &version, None),
        ] {
            assert!(matches!(result, Err(Error::Unavailable)));
        }
    }
}
