//! Recovery receipts for an interrupted apply. Completed applications require
//! a separate undo operation; this module cannot reopen or reverse them.
use super::*;
use research_agent_core::workspace::FileVersion;

/// Caller verifies actual disk/index state and holds the immediate transaction.
/// A frozen before version supplied by itself is not proof of restoration.
pub fn acknowledge_disk_restored_in(
    tx: &Transaction<'_>,
    scope: ApplicationScope,
    path: &str,
    actual: Option<&FileVersion>,
) -> Result<(), Error> {
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
    if file.target != ApplicationTarget::Disk {
        return Err(Error::Conflict);
    }
    let proposal = change_sets::read_file_operation_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )?;
    let change = proposal
        .changes
        .iter()
        .find(|change| change.path == path)
        .ok_or(Error::Conflict)?;
    if change.before.as_ref() != actual {
        return Err(Error::Conflict);
    }
    if file.phase == ApplicationPhase::Restored
        && ((saved.status == ApplicationStatus::RecoveryRequired
            && proposal.status == ChangeSetStatus::RecoveryRequired)
            || (saved.status == ApplicationStatus::RolledBack
                && proposal.status == ChangeSetStatus::Undone))
    {
        return Ok(());
    }
    // Recovery must be committed before physical restoration begins. This also
    // closes admission for still-pending files in a partly applied proposal.
    if saved.status != ApplicationStatus::RecoveryRequired
        || proposal.status != ChangeSetStatus::RecoveryRequired
        || !matches!(
            file.phase,
            ApplicationPhase::Dispatched
                | ApplicationPhase::Acknowledged
                | ApplicationPhase::Conflict
        )
    {
        return Err(Error::Conflict);
    }
    if tx.execute("update agent_proposal_application_files set phase='restored',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where application_id=?1 and relative_path=?2 and phase in ('dispatched','acknowledged','conflict')", params![scope.operation.to_string(),path])? != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

/// End only an interrupted apply whose dispatched effects have all been
/// restored. Pending files were never dispatched and need no invented receipt.
pub fn finish_rollback_in(tx: &mut Transaction<'_>, scope: ApplicationScope) -> Result<(), Error> {
    let saved = read_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )?;
    let proposal = change_sets::read_file_operation_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )?;
    if saved.status == ApplicationStatus::RolledBack && proposal.status == ChangeSetStatus::Undone {
        return Ok(());
    }
    if saved.status != ApplicationStatus::RecoveryRequired
        || proposal.status != ChangeSetStatus::RecoveryRequired
        || saved.files.iter().any(|file| {
            !matches!(
                file.phase,
                ApplicationPhase::Pending | ApplicationPhase::Restored
            )
        })
    {
        return Err(Error::Conflict);
    }
    let sp = tx.savepoint()?;
    if sp.execute("update agent_proposal_applications set status='rolled_back',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status='recovery_required'",[scope.operation.to_string()])? != 1
        || sp.execute("update agent_change_sets set status='undone' where id=?1 and status='recovery_required'",[scope.proposal.to_string()])? != 1 {
        return Err(Error::Conflict);
    }
    sp.commit()?;
    Ok(())
}
