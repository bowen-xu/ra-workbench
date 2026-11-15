//! Compensation receipts belong to undo, leaving the original apply immutable.
use super::*;
use applications::{ApplicationPhase, ApplicationTarget};
use research_agent_core::workspace::FileVersion;

/// Actual bytes and index must have been verified by the caller in this tx.
pub fn acknowledge_disk_restored_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
    actual: Option<&FileVersion>,
) -> Result<(), Error> {
    let saved = read_in(tx, original, operation)?.ok_or(Error::Conflict)?;
    let file = saved
        .application
        .files
        .iter()
        .find(|f| f.path == path)
        .ok_or(Error::Conflict)?;
    let change = saved
        .changes
        .iter()
        .find(|c| c.path == path)
        .ok_or(Error::Conflict)?;
    if file.target != ApplicationTarget::Disk || change.before.as_ref() != actual {
        return Err(Error::Conflict);
    }
    if file.phase == ApplicationPhase::Restored
        && matches!(
            saved.application.status,
            ApplicationStatus::RecoveryRequired | ApplicationStatus::RolledBack
        )
    {
        return Ok(());
    }
    if saved.application.status != ApplicationStatus::RecoveryRequired
        || !matches!(
            file.phase,
            ApplicationPhase::Dispatched | ApplicationPhase::Acknowledged
        )
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
    if tx.execute("update agent_proposal_application_files set phase='restored',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where application_id=?1 and relative_path=?2 and phase in ('dispatched','acknowledged')", params![operation.to_string(),path])? != 1 {return Err(Error::Conflict);}
    Ok(())
}

pub fn finish_rollback_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
) -> Result<(), Error> {
    let saved = read_in(tx, original, operation)?.ok_or(Error::Conflict)?;
    if saved.application.status == ApplicationStatus::RolledBack {
        return Ok(());
    }
    if saved.application.status != ApplicationStatus::RecoveryRequired
        || saved.application.files.iter().any(|f| {
            !matches!(
                f.phase,
                ApplicationPhase::Pending | ApplicationPhase::Restored
            )
        })
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
    if tx.execute("update agent_proposal_applications set status='rolled_back',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status='recovery_required'", [operation.to_string()])? != 1 {return Err(Error::Conflict);}
    Ok(())
}
