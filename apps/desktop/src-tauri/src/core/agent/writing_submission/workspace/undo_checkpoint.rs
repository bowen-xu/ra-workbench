//! Current file checkpoint CAS proves rebuilt ownership; old receipts stay frozen.
use super::*;
use crate::agent::repository::proposal_applications::{
    ApplicationFile, ApplicationPhase, ApplicationScope, ApplicationTarget,
};
use crate::agent::repository::proposal_undos::StoredUndo;
use crate::agent::writing_checkpoint_contracts::CheckpointRecord;
use crate::agent::writing_checkpoint_contracts::DocumentCheckpoint;
use crate::agent::writing_workspace::ReadCheckpoint;
use crate::ai::structured_resource_validation::Flow;
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "undo_checkpoint_tests.rs"]
mod tests;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UndoCheckpointVersion {
    pub path: String,
    pub revision: u64,
    pub state_version: u64,
}

pub(super) fn read_in(
    tx: &rusqlite::Connection,
    checkpoint_source: Option<ReadCheckpoint>,
    request: &WritingWorkspaceRequest,
    versions: &[UndoCheckpointVersion],
) -> Result<BTreeMap<String, DocumentCheckpoint>, Error> {
    read_with(request, versions, |manuscript, path| {
        let source =
            checkpoint_source.ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        source(tx, manuscript, path).map_err(|_| Flow::original(LeaseError::Conflict.into()))
    })
    .map_err(Flow::<Error, std::convert::Infallible>::into_static)
}

pub(crate) fn read_with<P>(
    request: &WritingWorkspaceRequest,
    versions: &[UndoCheckpointVersion],
    mut read_checkpoint: impl FnMut(Uuid, &str) -> Result<Option<CheckpointRecord>, Flow<Error, P>>,
) -> Result<BTreeMap<String, DocumentCheckpoint>, Flow<Error, P>> {
    let mut records = BTreeMap::new();
    if versions.len() > request.buffers.len() {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    for version in versions {
        let buffer = request
            .buffers
            .iter()
            .find(|buffer| buffer.path == version.path)
            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        let record = read_checkpoint(request.manuscript_id, &version.path)?
            .ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        if version.revision == 0
            || version.revision > 9_007_199_254_740_991
            || version.state_version > 9_007_199_254_740_991
            || record.revision != version.revision
            || record.checkpoint.version != version.state_version
            || record.checkpoint.content != buffer.text
            || record.checkpoint.saved_sha256 != buffer.expected_disk_sha256
            || record.checkpoint.save_intent.is_some()
            || records.contains_key(&version.path)
        {
            return Err(Flow::original(LeaseError::Conflict.into()));
        }
        records.insert(version.path.clone(), record.checkpoint);
    }
    Ok(records)
}

pub(super) fn authorize_rebuilt(
    checkpoint: &DocumentCheckpoint,
    original: ApplicationScope,
    request_id: Uuid,
    saved: &StoredUndo,
    file: &ApplicationFile,
) -> Result<(), Error> {
    let change = saved
        .changes
        .iter()
        .find(|change| change.path == file.path)
        .ok_or(LeaseError::Conflict)?;
    let before = change.before.as_ref().ok_or(LeaseError::Conflict)?;
    let source_matches = |operation: Uuid, hash: &str| {
        checkpoint.operations.iter().any(|source| {
            source.operation_id == operation.to_string()
                && source.conversation_id == original.conversation.to_string()
                && source.request_id == request_id.to_string()
                && source.result_sha256 == hash
        })
    };
    if !source_matches(original.operation, &before.sha256) {
        return Err(LeaseError::Conflict.into());
    }
    if file.phase == ApplicationPhase::Acknowledged {
        let ApplicationTarget::Editor {
            expected,
            result_revision: Some(revision),
        } = &file.target
        else {
            return Err(LeaseError::Conflict.into());
        };
        let receipt = checkpoint
            .editor_receipt
            .as_ref()
            .ok_or(LeaseError::Conflict)?;
        let after = change.after.as_ref().ok_or(LeaseError::Conflict)?;
        // A receipt is observed evidence from the departed session, not the new owner.
        if receipt.operation_id != saved.application.operation_id.to_string()
            || receipt.session_id != expected.session_id.to_string()
            || receipt.revision != revision.to_string()
            || receipt.text != checkpoint.content
            || !source_matches(saved.application.operation_id, &after.sha256)
        {
            return Err(LeaseError::Conflict.into());
        }
    }
    Ok(())
}

/// Current checkpoint CAS is evidence; the receipt retains its historical owner.
#[cfg(test)]
fn read_editor_in(
    tx: &rusqlite::Connection,
    checkpoint_source: Option<ReadCheckpoint>,
    original: ApplicationScope,
    current: &EditorObservation,
    proof: &UndoCheckpointVersion,
) -> Result<DocumentCheckpoint, Error> {
    read_editor_with(current, proof, || {
        let source =
            checkpoint_source.ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
        source(tx, original.manuscript, &current.path)
            .map_err(|_| Flow::original(LeaseError::Conflict.into()))
    })
    .map_err(Flow::<Error, std::convert::Infallible>::into_static)
}

pub(crate) fn read_editor_with<P>(
    current: &EditorObservation,
    proof: &UndoCheckpointVersion,
    read_checkpoint: impl FnOnce() -> Result<Option<CheckpointRecord>, Flow<Error, P>>,
) -> Result<DocumentCheckpoint, Flow<Error, P>> {
    let record = read_checkpoint()?.ok_or_else(|| Flow::original(LeaseError::Conflict.into()))?;
    if proof.path != current.path
        || proof.revision == 0
        || proof.revision > 9_007_199_254_740_991
        || proof.state_version > 9_007_199_254_740_991
        || record.revision != proof.revision
        || record.checkpoint.version != proof.state_version
        || record.checkpoint.content != current.text
        || record.checkpoint.save_intent.is_some()
    {
        return Err(Flow::original(LeaseError::Conflict.into()));
    }
    Ok(record.checkpoint)
}

pub(super) fn authorize_restored(
    checkpoint: &DocumentCheckpoint,
    original: ApplicationScope,
    request_id: Uuid,
    saved: &StoredUndo,
    file: &ApplicationFile,
    restoration: &crate::agent::repository::proposal_applications::EditorRestoration,
) -> Result<(), Error> {
    authorize_rebuilt(checkpoint, original, request_id, saved, file)?;
    let before = saved
        .changes
        .iter()
        .find(|change| change.path == file.path)
        .and_then(|change| change.before.as_ref())
        .ok_or(LeaseError::Conflict)?;
    let receipt = checkpoint
        .editor_receipt
        .as_ref()
        .ok_or(LeaseError::Conflict)?;
    if receipt.operation_id != restoration.id.to_string()
        || receipt.session_id != restoration.expected.session_id.to_string()
        || Some(receipt.revision.as_str())
            != restoration
                .result_revision
                .map(|r| r.to_string())
                .as_deref()
        || receipt.text != checkpoint.content
        || !checkpoint.operations.iter().any(|source| {
            source.operation_id == restoration.id.to_string()
                && source.conversation_id == original.conversation.to_string()
                && source.request_id == request_id.to_string()
                && source.result_sha256 == before.sha256
        })
    {
        return Err(LeaseError::Conflict.into());
    }
    Ok(())
}

/// Re-admission retains the same formal-file and index protection as the whole plan.
pub(super) fn check_editor_disk<S: super::restoration_source::RestorationSource>(
    store: &AppStore,
    tx: &rusqlite::Connection,
    original: ApplicationScope,
    saved: &StoredUndo,
    file: &ApplicationFile,
    checkpoint: &DocumentCheckpoint,
    source_reader: &mut S,
) -> Result<(), crate::ai::structured_resource_validation::Flow<Error, S::Pause>> {
    use crate::ai::structured_resource_validation::Flow;
    let disk = (|| -> Result<_, Error> {
        let source = store
            .manuscript_directory()
            .canonicalize()
            .map_err(|_| Error::Storage)?
            .join(original.manuscript.to_string())
            .join("source");
        let disk = crate::agent::workspace::DiskTarget::open(&source, &file.path)
            .map_err(|_| Error::RecoveryRequired)?
            .observe()
            .map_err(|_| Error::RecoveryRequired)?
            .ok_or(LeaseError::Conflict)?;
        let change = saved
            .changes
            .iter()
            .find(|c| c.path == file.path)
            .ok_or(LeaseError::Conflict)?;
        let before = change.before.as_ref().ok_or(LeaseError::Conflict)?;
        if checkpoint.saved_sha256 != disk.sha256
            || disk.executable != before.executable
            || (Some(&disk) != change.before.as_ref() && Some(&disk) != change.after.as_ref())
        {
            return Err(LeaseError::Conflict.into());
        }
        Ok(disk)
    })()
    .map_err(Flow::original)?;
    source_reader
        .index(&file.path)?
        .check_registered(tx, original.manuscript, &file.path, &disk)
        .map_err(|cause| {
            Flow::original(cause.map(|error| WritingSubmissionError::from(error).into()))
        })?;
    Ok(())
}
