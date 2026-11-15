//! Durable identity for a compensating editor transaction. No editor dispatch here.
use super::*;
use research_agent_core::workspace::EditorVersion;

#[derive(Debug, PartialEq, Eq)]
pub struct EditorRestoration {
    pub id: Uuid,
    pub expected: EditorVersion,
    pub result_revision: Option<Uuid>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum EditorRestorationAdmission {
    Created(EditorRestoration),
    Existing(EditorRestoration),
}

pub fn read_editor_restoration_in(
    db: &Connection,
    scope: ApplicationScope,
    path: &str,
) -> Result<Option<EditorRestoration>, Error> {
    // Scope validation precedes even an empty lookup.
    let saved = read_in(
        db,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )?;
    read_editor_restoration_record_in(db, &saved, path)
}

/// The native host must verify current editor bytes against frozen after before
/// admitting this intent and commit the transaction before dispatching restoration.
pub fn begin_editor_restoration_in(
    tx: &Transaction<'_>,
    scope: ApplicationScope,
    path: &str,
    restore_id: Uuid,
    expected: &EditorVersion,
) -> Result<EditorRestorationAdmission, Error> {
    let saved = read_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )?;
    begin_editor_restoration_record_in(tx, &saved, path, restore_id, expected)
}

/// Caller validates the operation-specific editor receipt's bytes against frozen
/// before. Both receipt and restored phase commit together, even if caller catches
/// an error and commits the enclosing transaction.
pub fn acknowledge_editor_restored_in(
    tx: &mut Transaction<'_>,
    scope: ApplicationScope,
    path: &str,
    restore_id: Uuid,
    actual: &EditorVersion,
) -> Result<(), Error> {
    let restoration = read_editor_restoration_in(tx, scope, path)?.ok_or(Error::Conflict)?;
    if restoration.id != restore_id
        || actual.session_id != restoration.expected.session_id
        || actual.revision.is_nil()
        || actual.revision == restoration.expected.revision
    {
        return Err(Error::Conflict);
    }
    let saved = read_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )?;
    let file = saved
        .files
        .iter()
        .find(|file| file.path == path)
        .ok_or(Error::Conflict)?;
    let proposal = change_sets::read_file_operation_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )?;
    if let Some(revision) = restoration.result_revision {
        return if revision == actual.revision
            && file.phase == ApplicationPhase::Restored
            && ((saved.status == ApplicationStatus::RecoveryRequired
                && proposal.status == ChangeSetStatus::RecoveryRequired)
                || (saved.status == ApplicationStatus::RolledBack
                    && proposal.status == ChangeSetStatus::Undone))
        {
            Ok(())
        } else {
            Err(Error::Conflict)
        };
    }
    if saved.status != ApplicationStatus::RecoveryRequired
        || proposal.status != ChangeSetStatus::RecoveryRequired
        || file.phase != ApplicationPhase::Acknowledged
    {
        return Err(Error::Conflict);
    }
    commit_editor_restoration_record_in(tx, scope.operation, path, actual.revision)
}
