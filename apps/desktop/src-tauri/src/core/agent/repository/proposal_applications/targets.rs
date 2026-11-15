//! Renew only undispatched CAS targets after a native whole-plan preflight.
use super::*;

pub(crate) fn rebind_pending_targets_in(
    tx: &mut Transaction<'_>,
    scope: ApplicationScope,
    targets: &BTreeMap<String, Option<EditorVersion>>,
) -> Result<(), Error> {
    let saved = read_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )?;
    if saved.files.len() != targets.len()
        || saved
            .files
            .iter()
            .any(|file| file.phase != ApplicationPhase::Pending)
    {
        return Err(Error::Conflict);
    }
    let sp = tx.savepoint()?;
    for file in saved.files {
        let editor = targets.get(&file.path).ok_or(Error::Conflict)?.as_ref();
        let previous = match &file.target {
            ApplicationTarget::Disk => None,
            ApplicationTarget::Editor { expected, .. } => Some(expected),
        };
        if previous == editor {
            continue;
        }
        // No change of ownership during an active application, nor a fallback
        // to disk when the old operation required an editor receipt.
        if saved.status != ApplicationStatus::RecoveryRequired
            || editor.is_none()
            || editor.is_some_and(|e| e.session_id.is_nil() || e.revision.is_nil())
        {
            return Err(Error::Conflict);
        }
        if sp.execute(
            "update agent_proposal_application_files set target_kind='editor',expected_editor_session=?3,expected_editor_revision=?4,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where application_id=?1 and relative_path=?2 and phase='pending'",
            params![scope.operation.to_string(), file.path, editor.map(|e|e.session_id.to_string()), editor.map(|e|e.revision.to_string())],
        )? != 1 {
            return Err(Error::Conflict);
        }
    }
    sp.commit()?;
    Ok(())
}
