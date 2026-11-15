//! Application admission only. Caller must preflight live sources and own the
//! immediate transaction; this module never writes files or dispatches editors.
use super::{
    change_sets::{self, ChangeSetStatus},
    execution_workspaces::Error,
};
use research_agent_core::workspace::{EditorVersion, FileChangeKind};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::BTreeMap;
use uuid::Uuid;

#[path = "proposal_applications/editor_restoration.rs"]
mod editor_restoration;
#[path = "proposal_applications/editor_restoration_record.rs"]
mod editor_restoration_record;
pub(crate) use editor_restoration_record::{
    begin_editor_restoration_record_in, commit_editor_restoration_record_in,
    read_editor_restoration_record_in,
};
#[path = "proposal_applications/progress.rs"]
mod progress;
pub(crate) use progress::{acknowledge_record_in, dispatch_record_in};
#[path = "proposal_applications/read.rs"]
mod read;
pub(crate) use read::read_record_in;
#[path = "proposal_applications/rollback.rs"]
mod rollback;
#[path = "proposal_applications/startup.rs"]
mod startup;
#[path = "proposal_applications/targets.rs"]
mod targets;
pub use editor_restoration::{
    EditorRestoration, EditorRestorationAdmission, acknowledge_editor_restored_in,
    begin_editor_restoration_in, read_editor_restoration_in,
};
pub use progress::{
    ApplicationScope, DispatchAdmission, FileReceipt, acknowledge_in, dispatch_in, finish_in,
    require_recovery_in, resume_in,
};
pub use read::{
    ApplicationFile, ApplicationPhase, ApplicationStatus, ApplicationTarget, StoredApplication,
    find_in, read_in,
};
pub use rollback::{acknowledge_disk_restored_in, finish_rollback_in};
pub use startup::recover_on_startup;
pub(crate) use targets::rebind_pending_targets_in;

#[derive(Debug, PartialEq, Eq)]
pub enum ApplicationAdmission {
    Created(Uuid),
    /// Reconcile/read the existing attempt; never treat this as new dispatch authority.
    Existing(Uuid),
}

pub fn exists_in(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
    operation: Uuid,
) -> Result<bool, Error> {
    if operation.is_nil() || proposal.is_nil() || conversation.is_nil() || manuscript.is_nil() {
        return Err(Error::Conflict);
    }
    let row: Option<(String, String)> = db
        .query_row(
            "select change_set_id,direction from agent_proposal_applications where id=?1",
            [operation.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((saved, direction)) = row else {
        return Ok(false);
    };
    if saved != proposal.to_string() || direction != "apply" {
        return Err(Error::Conflict);
    }
    let frozen = change_sets::read_file_operation_in(db, conversation, manuscript, proposal)?;
    let count: usize = db.query_row(
        "select count(*) from agent_proposal_application_files where application_id=?1",
        [operation.to_string()],
        |r| r.get(0),
    )?;
    if count != frozen.changes.len() {
        return Err(Error::Conflict);
    }
    let saved = read_record_in(db, proposal, operation)?;
    let files: BTreeMap<_, _> = saved
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    for change in frozen.changes {
        let file = files.get(change.path.as_str()).ok_or(Error::Conflict)?;
        let valid = match &file.target {
            ApplicationTarget::Disk => change
                .source
                .as_ref()
                .is_none_or(|source| source.editor.is_none()),
            ApplicationTarget::Editor { .. } => {
                change.kind == FileChangeKind::Modified
                    && change.before.is_some()
                    && change.after.is_some()
            }
        };
        if !valid {
            return Err(Error::Conflict);
        }
    }
    Ok(true)
}

/// Savepoint ensures partial target insertion cannot survive an outer commit.
/// Preflight must run in this transaction immediately before a new admission.
#[cfg(test)]
pub fn begin_in(
    tx: &mut Transaction<'_>,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
    operation: Uuid,
) -> Result<ApplicationAdmission, Error> {
    let frozen = change_sets::read_in(tx, conversation, manuscript, proposal)?;
    let targets = frozen
        .changes
        .iter()
        .map(|change| {
            (
                change.path.clone(),
                change
                    .source
                    .as_ref()
                    .and_then(|source| source.editor.clone()),
            )
        })
        .collect();
    begin_with_targets_in(tx, conversation, manuscript, proposal, operation, &targets)
}

/// Targets must come from a native whole-plan preflight in this transaction.
/// This seals current CAS identities without rewriting historical provenance.
pub(crate) fn begin_with_targets_in(
    tx: &mut Transaction<'_>,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
    operation: Uuid,
    targets: &BTreeMap<String, Option<EditorVersion>>,
) -> Result<ApplicationAdmission, Error> {
    if exists_in(tx, conversation, manuscript, proposal, operation)? {
        return Ok(ApplicationAdmission::Existing(operation));
    }
    let frozen = change_sets::read_in(tx, conversation, manuscript, proposal)?;
    if frozen.status != ChangeSetStatus::Pending {
        return Err(Error::Conflict);
    }
    if targets.len() != frozen.changes.len() {
        return Err(Error::Conflict);
    }
    let sp = tx.savepoint()?;
    sp.execute(
        "insert into agent_proposal_applications(id,change_set_id,direction) values(?1,?2,'apply')",
        params![operation.to_string(), proposal.to_string()],
    )?;
    for change in frozen.changes {
        let editor = targets.get(&change.path).ok_or(Error::Conflict)?.as_ref();
        if editor.is_some_and(|version| version.session_id.is_nil() || version.revision.is_nil())
            || (editor.is_none()
                && change
                    .source
                    .as_ref()
                    .is_some_and(|source| source.editor.is_some()))
            || (editor.is_some() && change.kind != FileChangeKind::Modified)
        {
            return Err(Error::Conflict);
        }
        sp.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind,expected_editor_session,expected_editor_revision) values(?1,?2,?3,?4,?5,?6)",params![operation.to_string(),proposal.to_string(),change.path,if editor.is_some(){"editor"}else{"disk"},editor.map(|e|e.session_id.to_string()),editor.map(|e|e.revision.to_string())])?;
    }
    sp.execute("update agent_proposal_applications set status='applying',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1",[operation.to_string()])?;
    if sp.execute(
        "update agent_change_sets set status='applying' where id=?1 and status='pending'",
        [proposal.to_string()],
    )? != 1
    {
        return Err(Error::Conflict);
    }
    sp.commit()?;
    Ok(ApplicationAdmission::Created(operation))
}
