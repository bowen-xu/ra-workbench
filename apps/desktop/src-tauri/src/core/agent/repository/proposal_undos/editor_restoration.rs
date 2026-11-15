//! Undo compensation owns business checks; apply and undo share journal I/O.
use super::*;
use applications::{ApplicationPhase, EditorRestoration, EditorRestorationAdmission};
use research_agent_core::workspace::EditorVersion;

pub fn read_editor_restoration_in(
    db: &Connection,
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
) -> Result<Option<EditorRestoration>, Error> {
    let saved = read_in(db, original, operation)?.ok_or(Error::Conflict)?;
    applications::read_editor_restoration_record_in(db, &saved.application, path)
}

/// Host verifies current bytes against the inverse plan's after version before
/// committing this intent. Existing intent is history, never dispatch permission.
pub fn begin_editor_restoration_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
    restore_id: Uuid,
    expected: &EditorVersion,
) -> Result<EditorRestorationAdmission, Error> {
    if restore_id == original.operation {
        return Err(Error::Conflict);
    }
    let saved = read_in(tx, original, operation)?.ok_or(Error::Conflict)?;
    if let Some(restoration) =
        applications::read_editor_restoration_record_in(tx, &saved.application, path)?
    {
        return if restoration.id == restore_id {
            Ok(EditorRestorationAdmission::Existing(restoration))
        } else {
            Err(Error::Conflict)
        };
    }
    let file = saved
        .application
        .files
        .iter()
        .find(|f| f.path == path)
        .ok_or(Error::Conflict)?;
    if file.phase != ApplicationPhase::Acknowledged
        || !matches!(&file.target,
        ApplicationTarget::Editor { expected: before, result_revision: Some(revision) }
        if before.session_id != expected.session_id || *revision == expected.revision)
    {
        return Err(Error::Conflict);
    }
    if change_sets::read_file_operation_in(
        tx,
        original.conversation,
        original.manuscript,
        original.proposal,
    )?
    .status
        != change_sets::ChangeSetStatus::Applied
    {
        return Err(Error::Conflict);
    }
    require_recovery_in(tx, original, operation)?;
    applications::begin_editor_restoration_record_in(
        tx,
        &saved.application,
        path,
        restore_id,
        expected,
    )
}

/// Host validates the operation-specific receipt bytes against inverse before.
/// A successful undo is never compensatable through this interrupted-operation API.
pub fn acknowledge_editor_restored_in(
    tx: &mut Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
    restore_id: Uuid,
    actual: &EditorVersion,
) -> Result<(), Error> {
    let saved = read_in(tx, original, operation)?.ok_or(Error::Conflict)?;
    let restoration =
        applications::read_editor_restoration_record_in(tx, &saved.application, path)?
            .ok_or(Error::Conflict)?;
    if restoration.id != restore_id
        || actual.session_id != restoration.expected.session_id
        || actual.revision.is_nil()
        || actual.revision == restoration.expected.revision
    {
        return Err(Error::Conflict);
    }
    let file = saved
        .application
        .files
        .iter()
        .find(|f| f.path == path)
        .ok_or(Error::Conflict)?;
    if let Some(revision) = restoration.result_revision {
        return if revision == actual.revision
            && file.phase == ApplicationPhase::Restored
            && matches!(
                saved.application.status,
                ApplicationStatus::RecoveryRequired | ApplicationStatus::RolledBack
            ) {
            Ok(())
        } else {
            Err(Error::Conflict)
        };
    }
    if saved.application.status != ApplicationStatus::RecoveryRequired
        || file.phase != ApplicationPhase::Acknowledged
        || change_sets::read_file_operation_in(
            tx,
            original.conversation,
            original.manuscript,
            original.proposal,
        )?
        .status
            != change_sets::ChangeSetStatus::Applied
    {
        return Err(Error::Conflict);
    }
    applications::commit_editor_restoration_record_in(tx, operation, path, actual.revision)
}
