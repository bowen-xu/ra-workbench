//! Rebuilt apply compensation uses current checkpoint CAS and immutable old receipts.
use super::*;
use crate::agent::repository::proposal_applications::{
    ApplicationFile, ApplicationScope, ApplicationTarget, EditorRestoration,
};
use crate::agent::writing_checkpoint_contracts::DocumentCheckpoint;
use research_agent_core::workspace::FileChange;

fn source_matches(
    checkpoint: &DocumentCheckpoint,
    scope: ApplicationScope,
    request: Uuid,
    operation: Uuid,
    hash: &str,
) -> bool {
    checkpoint.operations.iter().any(|source| {
        source.operation_id == operation.to_string()
            && source.conversation_id == scope.conversation.to_string()
            && source.request_id == request.to_string()
            && source.result_sha256 == hash
    })
}
fn require_original_source(
    checkpoint: &DocumentCheckpoint,
    scope: ApplicationScope,
    request: Uuid,
    change: &FileChange,
) -> Result<(), Error> {
    let after = change.after.as_ref().ok_or(LeaseError::Conflict)?;
    if !source_matches(checkpoint, scope, request, scope.operation, &after.sha256) {
        return Err(LeaseError::Conflict.into());
    }
    Ok(())
}
pub(super) fn authorize_rebuilt(
    checkpoint: &DocumentCheckpoint,
    scope: ApplicationScope,
    request: Uuid,
    change: &FileChange,
    file: &ApplicationFile,
) -> Result<(), Error> {
    require_original_source(checkpoint, scope, request, change)?;
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
    if receipt.operation_id != scope.operation.to_string()
        || receipt.session_id != expected.session_id.to_string()
        || receipt.revision != revision.to_string()
        || receipt.text != checkpoint.content
    {
        return Err(LeaseError::Conflict.into());
    }
    Ok(())
}
pub(super) fn authorize_restored(
    checkpoint: &DocumentCheckpoint,
    scope: ApplicationScope,
    request: Uuid,
    change: &FileChange,
    restoration: &EditorRestoration,
) -> Result<(), Error> {
    require_original_source(checkpoint, scope, request, change)?;
    let before = change.before.as_ref().ok_or(LeaseError::Conflict)?;
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
        || !source_matches(checkpoint, scope, request, restoration.id, &before.sha256)
    {
        return Err(LeaseError::Conflict.into());
    }
    Ok(())
}
/// Formal disk can still contain the captured source, even when frozen before
/// came from an unsaved editor buffer. Only the exact applied result is an exception.
pub(super) fn check_editor_disk<S: super::restoration_source::RestorationSource>(
    store: &AppStore,
    tx: &rusqlite::Connection,
    scope: ApplicationScope,
    change: &FileChange,
    checkpoint: &DocumentCheckpoint,
    source_reader: &mut S,
) -> Result<(), crate::ai::structured_resource_validation::Flow<Error, S::Pause>> {
    use crate::ai::structured_resource_validation::Flow;
    let disk = (|| -> Result<_, Error> {
        let source = store
            .manuscript_directory()
            .canonicalize()
            .map_err(|_| Error::Storage)?
            .join(scope.manuscript.to_string())
            .join("source");
        let disk = crate::agent::workspace::DiskTarget::open(&source, &change.path)
            .map_err(|_| Error::RecoveryRequired)?
            .observe()
            .map_err(|_| Error::RecoveryRequired)?
            .ok_or(LeaseError::Conflict)?;
        let recorded = change.source.as_ref().ok_or(LeaseError::Conflict)?;
        let before = change.before.as_ref().ok_or(LeaseError::Conflict)?;
        if checkpoint.saved_sha256 != disk.sha256
            || disk.executable != before.executable
            || recorded.resource_id != format!("{}/{}", scope.manuscript, change.path)
            || (recorded.revision != format!("sha256:{}", disk.sha256)
                && change.after.as_ref() != Some(&disk))
        {
            return Err(LeaseError::Conflict.into());
        }
        Ok(disk)
    })()
    .map_err(Flow::original)?;
    source_reader
        .index(&change.path)?
        .check_registered(tx, scope.manuscript, &change.path, &disk)
        .map_err(|cause| {
            Flow::original(cause.map(|error| WritingSubmissionError::from(error).into()))
        })?;
    Ok(())
}
