//! Undo transitions use inverse receipts, without reopening the original apply.
use super::*;
use applications::{ApplicationPhase, DispatchAdmission, FileReceipt};

fn required(
    db: &Connection,
    original: ApplicationScope,
    operation: Uuid,
) -> Result<StoredUndo, Error> {
    read_in(db, original, operation)?.ok_or(Error::Conflict)
}

/// Cancel only when no effect was dispatched. Never fabricate restoration.
pub fn cancel_pending_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
) -> Result<(), Error> {
    let saved = required(tx, original, operation)?;
    if saved.application.status == ApplicationStatus::RolledBack {
        return Ok(());
    }
    if !matches!(
        saved.application.status,
        ApplicationStatus::Prepared
            | ApplicationStatus::Applying
            | ApplicationStatus::RecoveryRequired
    ) || saved
        .application
        .files
        .iter()
        .any(|file| file.phase != ApplicationPhase::Pending)
    {
        return Err(Error::Conflict);
    }
    let proposal = change_sets::read_file_operation_in(
        tx,
        original.conversation,
        original.manuscript,
        original.proposal,
    )?;
    if proposal.status != change_sets::ChangeSetStatus::Applied {
        return Err(Error::Conflict);
    }
    if tx.execute("update agent_proposal_applications set status='rolled_back',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status in ('prepared','applying','recovery_required')", [operation.to_string()])? != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

/// Full current-source preflight must precede this transition in the same tx.
pub fn start_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
) -> Result<(), Error> {
    let saved = required(tx, original, operation)?;
    if saved.application.status != ApplicationStatus::Prepared {
        return Err(Error::Conflict);
    }
    let proposal = change_sets::read_file_operation_in(
        tx,
        original.conversation,
        original.manuscript,
        original.proposal,
    )?;
    if proposal.status != change_sets::ChangeSetStatus::Applied {
        return Err(Error::Conflict);
    }
    if tx.execute("update agent_proposal_applications set status='applying',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status='prepared'", [operation.to_string()])? != 1 { return Err(Error::Conflict); }
    Ok(())
}

/// Caller must verify the current before/after versions for every target in this
/// transaction. Only fully reconciled progress can admit further dispatch.
pub fn resume_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
) -> Result<(), Error> {
    let saved = required(tx, original, operation)?;
    // An editor restoration intent may already have changed the live buffer
    // without an ACK. Its recovery must finish before any forward undo resumes.
    let restoring: bool = tx.query_row(
        "select exists(select 1 from agent_editor_restorations where application_id=?1)",
        [operation.to_string()],
        |row| row.get(0),
    )?;
    if saved.application.status != ApplicationStatus::RecoveryRequired
        || restoring
        || !saved
            .application
            .files
            .iter()
            .any(|f| f.phase == ApplicationPhase::Pending)
        || saved.application.files.iter().any(|f| {
            !matches!(
                f.phase,
                ApplicationPhase::Pending | ApplicationPhase::Acknowledged
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
    if tx.execute("update agent_proposal_applications set status='applying',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status='recovery_required'", [operation.to_string()])? != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub fn dispatch_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
    path: &str,
) -> Result<DispatchAdmission, Error> {
    let saved = required(tx, original, operation)?;
    applications::dispatch_record_in(tx, &saved.application, path)
}

pub fn acknowledge_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
    receipt: &FileReceipt,
) -> Result<(), Error> {
    let saved = required(tx, original, operation)?;
    let change = saved
        .changes
        .iter()
        .find(|c| c.path == receipt.path)
        .ok_or(Error::Conflict)?;
    applications::acknowledge_record_in(tx, &saved.application, change, receipt)
}

pub fn require_recovery_in(
    tx: &Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
) -> Result<(), Error> {
    let saved = required(tx, original, operation)?;
    if saved.application.status == ApplicationStatus::RecoveryRequired {
        return Ok(());
    }
    if saved.application.status != ApplicationStatus::Applying {
        return Err(Error::Conflict);
    }
    if tx.execute("update agent_proposal_applications set status='recovery_required',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status='applying'", [operation.to_string()])? != 1 { return Err(Error::Conflict); }
    Ok(())
}

/// Only exact confirmed inverse effects finish; all changes commit atomically.
pub fn finish_in(
    tx: &mut Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
) -> Result<(), Error> {
    let saved = required(tx, original, operation)?;
    let proposal = change_sets::read_file_operation_in(
        tx,
        original.conversation,
        original.manuscript,
        original.proposal,
    )?;
    if saved.application.status == ApplicationStatus::Applied
        && proposal.status == change_sets::ChangeSetStatus::Undone
    {
        return Ok(());
    }
    if !matches!(
        saved.application.status,
        ApplicationStatus::Applying | ApplicationStatus::RecoveryRequired
    ) || saved
        .application
        .files
        .iter()
        .any(|f| f.phase != ApplicationPhase::Acknowledged)
        || proposal.status != change_sets::ChangeSetStatus::Applied
    {
        return Err(Error::Conflict);
    }
    let sp = tx.savepoint()?;
    if sp.execute("update agent_proposal_applications set status='applied',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status in ('applying','recovery_required')", [operation.to_string()])? != 1
        || sp.execute("update agent_change_sets set status='undone' where id=?1 and status='applied'", [original.proposal.to_string()])? != 1 { return Err(Error::Conflict); }
    sp.commit()?;
    Ok(())
}
