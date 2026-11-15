//! Durable coordinator transitions, never a renderer-facing write interface.
use super::*;
use research_agent_core::workspace::{EditorVersion, FileVersion};

#[derive(Clone, Copy)]
pub struct ApplicationScope {
    pub conversation: Uuid,
    pub manuscript: Uuid,
    pub proposal: Uuid,
    pub operation: Uuid,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DispatchAdmission {
    /// Commit the intent before executing. Still requires live-source validation.
    NewIntent,
    /// Read/reconcile; this never permits another dispatch.
    Existing(ApplicationPhase),
}

/// The coordinator derives this from actual disk bytes or the validated editor
/// receipt. Equality with frozen metadata alone is not proof of a real write.
pub struct FileReceipt {
    pub path: String,
    pub after: Option<FileVersion>,
    pub editor: Option<EditorVersion>,
}

fn read(tx: &Connection, scope: ApplicationScope) -> Result<StoredApplication, Error> {
    read_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
        scope.operation,
    )
}

/// Caller holds an immediate transaction and exclusive execution ownership.
/// A crash after commit requires reconciliation, even if no dispatch occurred.
pub fn dispatch_in(
    tx: &Transaction<'_>,
    scope: ApplicationScope,
    path: &str,
) -> Result<DispatchAdmission, Error> {
    let saved = read(tx, scope)?;
    dispatch_record_in(tx, &saved, path)
}

/// Reopen an interrupted application only when no file was ever dispatched.
/// Startup retires `applying` ownership as `recovery_required`; this transition
/// is the explicit, preflighted way to resume that exact operation. A pending
/// file is the only state that can be retried: any dispatched/acknowledged file
/// requires reconciliation and must never be replayed.
pub fn resume_in(tx: &mut Transaction<'_>, scope: ApplicationScope) -> Result<(), Error> {
    let saved = read(tx, scope)?;
    if saved.files.is_empty()
        || saved
            .files
            .iter()
            .any(|file| file.phase != ApplicationPhase::Pending)
    {
        return Err(Error::Conflict);
    }
    let proposal = change_sets::read_file_operation_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )?;
    match (saved.status, proposal.status) {
        (ApplicationStatus::Applying, ChangeSetStatus::Applying) => Ok(()),
        (ApplicationStatus::RecoveryRequired, ChangeSetStatus::RecoveryRequired) => {
            let sp = tx.savepoint()?;
            if sp.execute(
                "update agent_proposal_applications set status='applying',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status='recovery_required'",
                [scope.operation.to_string()],
            )? != 1
                || sp.execute(
                    "update agent_change_sets set status='applying' where id=?1 and status='recovery_required'",
                    [scope.proposal.to_string()],
                )? != 1
            {
                return Err(Error::Conflict);
            }
            sp.commit()?;
            Ok(())
        }
        _ => Err(Error::Conflict),
    }
}

/// Direction-specific caller validates the frozen plan in this transaction.
pub(crate) fn dispatch_record_in(
    tx: &Transaction<'_>,
    saved: &StoredApplication,
    path: &str,
) -> Result<DispatchAdmission, Error> {
    let file = saved
        .files
        .iter()
        .find(|file| file.path == path)
        .ok_or(Error::Conflict)?;
    if matches!(
        file.phase,
        ApplicationPhase::Dispatched | ApplicationPhase::Acknowledged
    ) {
        return Ok(DispatchAdmission::Existing(file.phase));
    }
    if saved.status != ApplicationStatus::Applying || file.phase != ApplicationPhase::Pending {
        return Err(Error::Conflict);
    }
    if tx.execute(
        "update agent_proposal_application_files set phase='dispatched',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where application_id=?1 and relative_path=?2 and phase='pending'",
        params![saved.operation_id.to_string(), path],
    )? != 1 {
        return Err(Error::Conflict);
    }
    Ok(DispatchAdmission::NewIntent)
}

/// Exact repeated ACKs are idempotent, changed receipts and skipped dispatch are
/// rejected. Caller verifies the real side effect and commits before reporting it.
pub fn acknowledge_in(
    tx: &Transaction<'_>,
    scope: ApplicationScope,
    receipt: &FileReceipt,
) -> Result<(), Error> {
    let saved = read(tx, scope)?;
    let proposal = change_sets::read_file_operation_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )?;
    let change = proposal
        .changes
        .iter()
        .find(|change| change.path == receipt.path)
        .ok_or(Error::Conflict)?;
    acknowledge_record_in(tx, &saved, change, receipt)
}

/// Shared receipt rules; caller supplies the direction-specific frozen change.
pub(crate) fn acknowledge_record_in(
    tx: &Transaction<'_>,
    saved: &StoredApplication,
    change: &research_agent_core::workspace::FileChange,
    receipt: &FileReceipt,
) -> Result<(), Error> {
    let file = saved
        .files
        .iter()
        .find(|file| file.path == receipt.path)
        .ok_or(Error::Conflict)?;
    if change.path != receipt.path || change.after != receipt.after {
        return Err(Error::Conflict);
    }
    let result_revision = match (&file.target, &receipt.editor) {
        (ApplicationTarget::Disk, None) => None,
        (ApplicationTarget::Editor { expected, .. }, Some(actual))
            if actual.session_id == expected.session_id
                && !actual.revision.is_nil()
                && actual.revision != expected.revision =>
        {
            Some(actual.revision)
        }
        _ => return Err(Error::Conflict),
    };
    if file.phase == ApplicationPhase::Acknowledged {
        let saved_revision = match file.target {
            ApplicationTarget::Disk => None,
            ApplicationTarget::Editor {
                result_revision, ..
            } => result_revision,
        };
        return if saved_revision == result_revision {
            Ok(())
        } else {
            Err(Error::Conflict)
        };
    }
    if !matches!(
        saved.status,
        ApplicationStatus::Applying | ApplicationStatus::RecoveryRequired
    ) || file.phase != ApplicationPhase::Dispatched
    {
        return Err(Error::Conflict);
    }
    if tx.execute(
        "update agent_proposal_application_files set phase='acknowledged',result_editor_revision=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where application_id=?1 and relative_path=?2 and phase='dispatched'",
        params![saved.operation_id.to_string(), receipt.path, result_revision.map(|id|id.to_string())],
    )? != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

/// Stop new dispatch without erasing known per-file progress. Atomic with the
/// proposal status, including when an outer caller commits after an error.
pub fn require_recovery_in(tx: &mut Transaction<'_>, scope: ApplicationScope) -> Result<(), Error> {
    set_terminal_in(tx, scope, false)
}

/// Only a completely acknowledged plan can finish. This records an observed
/// application, not a guarantee that users have not subsequently edited it.
pub fn finish_in(tx: &mut Transaction<'_>, scope: ApplicationScope) -> Result<(), Error> {
    // Once compensation has been admitted, late original receipts must not
    // finish the apply while an editor restoration may be in flight.
    if tx.query_row(
        "select exists(select 1 from agent_editor_restorations where application_id=?1)",
        [scope.operation.to_string()],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(Error::Conflict);
    }
    set_terminal_in(tx, scope, true)
}

fn set_terminal_in(
    tx: &mut Transaction<'_>,
    scope: ApplicationScope,
    finish: bool,
) -> Result<(), Error> {
    let saved = read(tx, scope)?;
    let proposal = change_sets::read_file_operation_in(
        tx,
        scope.conversation,
        scope.manuscript,
        scope.proposal,
    )?;
    let (target, proposal_target, sql_target) = if finish {
        (
            ApplicationStatus::Applied,
            ChangeSetStatus::Applied,
            "applied",
        )
    } else {
        (
            ApplicationStatus::RecoveryRequired,
            ChangeSetStatus::RecoveryRequired,
            "recovery_required",
        )
    };
    if saved.status == target && proposal.status == proposal_target {
        return Ok(());
    }
    // A late exact original ACK remains historical after a separately confirmed
    // undo. Never reopen the original or change the proposal back to applied.
    if finish && saved.status == ApplicationStatus::Applied && proposal.status == ChangeSetStatus::Undone
        && tx.query_row("select exists(select 1 from agent_proposal_undos u join agent_proposal_applications a on a.id=u.operation_id where u.original_operation_id=?1 and a.status='applied')", [scope.operation.to_string()], |r| r.get::<_, bool>(0))?
    { return Ok(()); }
    if !matches!(
        saved.status,
        ApplicationStatus::Applying | ApplicationStatus::RecoveryRequired
    ) || !matches!(
        proposal.status,
        ChangeSetStatus::Applying | ChangeSetStatus::RecoveryRequired
    ) || (finish
        && saved
            .files
            .iter()
            .any(|file| file.phase != ApplicationPhase::Acknowledged))
    {
        return Err(Error::Conflict);
    }
    let sp = tx.savepoint()?;
    if sp.execute("update agent_proposal_applications set status=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1 and status in ('applying','recovery_required')", params![scope.operation.to_string(),sql_target])? != 1
        || sp.execute("update agent_change_sets set status=?2 where id=?1 and status in ('applying','recovery_required')", params![scope.proposal.to_string(),sql_target])? != 1 {
        return Err(Error::Conflict);
    }
    sp.commit()?;
    Ok(())
}
