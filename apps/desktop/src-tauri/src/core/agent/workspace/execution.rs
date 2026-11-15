//! Persistent writer intent is outside the sandbox's writable work directory.
//! A dropped/failed guard never clears it: recovery must establish quiescence.
use super::{LeaseError, storage};
use crate::agent::{
    instructions::directory::Directory,
    process::{
        ProcessError, ProcessLimits, ProcessOutcome, ProcessRetention, ProcessSession, ProcessSpec,
        TerminalSize,
    },
};
use std::{
    ffi::OsStr,
    fs::File,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) const MARKER: &str = "execution.json";
pub struct WorkspaceExecution {
    directory: Directory,
    _lock: Arc<super::lease::OwnershipLock>,
    marker: File,
    work: PathBuf,
    work_identity: (u64, u64),
}
impl WorkspaceExecution {
    pub(super) fn begin(
        directory: Directory,
        lock: Arc<super::lease::OwnershipLock>,
        work: PathBuf,
    ) -> Result<Self, LeaseError> {
        if directory.config_file(MARKER)?.is_some() {
            return Err(LeaseError::Busy);
        }
        let work_identity = Directory::root(&work)?.identity()?;
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schemaVersion": 1, "executionId": uuid::Uuid::new_v4()}),
        )
        .map_err(|_| LeaseError::Invalid)?;
        let marker = storage::create_file(&directory, MARKER, &bytes, false)?;
        Ok(Self {
            directory,
            _lock: lock,
            marker,
            work,
            work_identity,
        })
    }
    /// Revalidate/bind this cwd at launch; this path does not authorize execution.
    pub fn working_directory(&self) -> &Path {
        &self.work
    }
    /// Consume into the process driver. A trusted native host must authorize and
    /// sandbox spec before calling. Startup failure retains intent for recovery.
    pub fn spawn(
        self,
        spec: ProcessSpec,
        limits: ProcessLimits,
        terminal: Option<TerminalSize>,
    ) -> Result<ProcessSession, ProcessError> {
        self.validate_cwd(&spec)?;
        ProcessSession::spawn_retained(spec, limits, terminal, Box::new(self))
    }
    /// Caller-driven hook supervision shares this exact lease and persistent
    /// intent; no background session driver or second workspace owner is created.
    pub(crate) fn spawn_hook(
        self,
        spec: ProcessSpec,
        limits: ProcessLimits,
    ) -> Result<
        (
            crate::agent::process::SupervisedProcess,
            crate::agent::process::PipeInput,
        ),
        ProcessError,
    > {
        self.validate_cwd(&spec)?;
        let (process, input) = crate::agent::process::SupervisedProcess::spawn(spec, limits)?;
        Ok((process.retain(Box::new(self)), input))
    }
    fn validate_cwd(&self, spec: &ProcessSpec) -> Result<(), ProcessError> {
        let actual = Directory::root(&self.work).and_then(|directory| directory.identity());
        if spec.directory != self.work || actual.ok() != Some(self.work_identity) {
            return Err(ProcessError::Invalid);
        }
        Ok(())
    }
    /// Blocking-worker only. The trusted native host must supply this execution's
    /// actual outcome, never a model/IPC-provided assertion. A result for another
    /// process is not admissible; binding is owned by the execution host.
    fn release_after_exit(self, outcome: &ProcessOutcome) -> Result<(), LeaseError> {
        if !outcome.drained || outcome.exit.is_none() {
            return Err(LeaseError::Incomplete);
        }
        let current = self
            .directory
            .config_file(MARKER)?
            .ok_or(LeaseError::Conflict)?
            .metadata()?;
        let held = self.marker.metadata()?;
        if (current.dev(), current.ino()) != (held.dev(), held.ino()) {
            return Err(LeaseError::Conflict);
        }
        storage::unlink_file(&self.directory, OsStr::new(MARKER))?;
        storage::sync(&self.directory)
    }
}

impl ProcessRetention for WorkspaceExecution {
    fn release(self: Box<Self>, outcome: &ProcessOutcome) -> Result<(), ProcessError> {
        self.release_after_exit(outcome)
            .map_err(|_| ProcessError::ResourceRelease)
    }
}

#[cfg(test)]
#[path = "execution/tests.rs"]
mod tests;
