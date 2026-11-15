//! Finite request-local reads for the original compensation checks.
use super::Error;
use crate::agent::writing_workspace::{WritingWorkspaceSource, index::FileIndex};
use crate::{
    agent::{workspace::LeaseError, writing_checkpoint_contracts::CheckpointRecord},
    ai::structured_resource_validation::Flow,
};
use rusqlite::Connection;
use std::convert::Infallible;
use uuid::Uuid;

pub(crate) trait RestorationSource {
    type Pause;
    fn checkpoint(
        &mut self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
    ) -> Result<Option<CheckpointRecord>, Flow<Error, Self::Pause>>;
    fn index(&mut self, path: &str) -> Result<FileIndex, Flow<Error, Self::Pause>>;
}

impl RestorationSource for Option<WritingWorkspaceSource> {
    type Pause = Infallible;
    fn checkpoint(
        &mut self,
        db: &Connection,
        manuscript: Uuid,
        path: &str,
    ) -> Result<Option<CheckpointRecord>, Flow<Error, Infallible>> {
        let source = self.ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        (source.read_checkpoint)(db, manuscript, path)
            .map_err(|_| Flow::original(LeaseError::Conflict.into()))
    }
    fn index(&mut self, _path: &str) -> Result<FileIndex, Flow<Error, Infallible>> {
        Ok(FileIndex::Static(*self))
    }
}
