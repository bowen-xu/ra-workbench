//! Terminal journals remain discoverable only with an unsaved, exact host receipt.
use super::{Error, Scope};
use crate::agent::repository::{change_sets, proposal_applications as applications};
use applications::{ApplicationPhase, ApplicationStatus, ApplicationTarget, StoredApplication};
use research_agent_core::workspace::FileChange;
use sha2::{Digest, Sha256};

pub(super) fn validate_with<P>(
    db: &rusqlite::Connection,
    scope: &Scope,
    application: &StoredApplication,
    changes: &[FileChange],
    mut read_checkpoint: impl FnMut(
        &str,
    ) -> Result<
        Option<crate::agent::writing_checkpoint_contracts::CheckpointRecord>,
        crate::ai::structured_resource_validation::Flow<Error, P>,
    >,
) -> Result<(), crate::ai::structured_resource_validation::Flow<Error, P>> {
    use crate::ai::structured_resource_validation::Flow;
    let status = application.status;
    if !matches!(
        status,
        ApplicationStatus::Applied | ApplicationStatus::RolledBack
    ) {
        return Ok(());
    }
    let proposal = change_sets::read_file_operation_in(
        db,
        scope.conversation_id,
        scope.manuscript_id,
        scope.proposal_id,
    )
    .map_err(|error| Flow::original(super::native::error(error.into())))?;
    let mut pending = false;
    for file in &application.files {
        let ApplicationTarget::Editor {
            expected,
            result_revision,
        } = &file.target
        else {
            continue;
        };
        if status == ApplicationStatus::RolledBack && file.phase != ApplicationPhase::Restored {
            continue;
        }
        let restoration = if status == ApplicationStatus::RolledBack {
            Some(
                applications::read_editor_restoration_record_in(db, application, &file.path)
                    .map_err(|error| Flow::original(super::native::error(error.into())))?
                    .ok_or_else(|| Flow::original(Error::Conflict))?,
            )
        } else {
            None
        };
        let (operation, session, revision) = match &restoration {
            Some(value) => (value.id, value.expected.session_id, value.result_revision),
            None => (
                application.operation_id,
                expected.session_id,
                *result_revision,
            ),
        };
        let Some(record) = read_checkpoint(&file.path)? else {
            continue;
        };
        let checkpoint = record.checkpoint;
        let Some(receipt) = &checkpoint.editor_receipt else {
            continue;
        };
        // A later admitted operation owns its own receipt and save boundary.
        if receipt.operation_id != operation.to_string() {
            continue;
        }
        let change = changes
            .iter()
            .find(|change| change.path == file.path)
            .ok_or_else(|| Flow::original(Error::Conflict))?;
        let version = if restoration.is_some() {
            &change.before
        } else {
            &change.after
        };
        let hash = format!("{:x}", Sha256::digest(receipt.text.as_bytes()));
        if receipt.session_id != session.to_string()
            || Some(receipt.revision.as_str())
                != revision.as_ref().map(|value| value.to_string()).as_deref()
            || version.as_ref().is_none_or(|value| {
                value.sha256 != hash || value.byte_size != receipt.text.len() as u64
            })
            || !checkpoint.operations.iter().any(|source| {
                source.operation_id == operation.to_string()
                    && source.conversation_id == scope.conversation_id.to_string()
                    && source.request_id == proposal.binding.request_id.to_string()
            })
            || !checkpoint.operations.iter().any(|source| {
                source.operation_id == scope.operation_id.to_string()
                    && source.conversation_id == scope.conversation_id.to_string()
                    && source.request_id == proposal.binding.request_id.to_string()
            })
        {
            return Err(Flow::original(Error::Conflict));
        }
        pending |= checkpoint.saved_version < receipt.state_version.unwrap_or(checkpoint.version);
    }
    if pending {
        Ok(())
    } else {
        Err(Flow::original(Error::Conflict))
    }
}
