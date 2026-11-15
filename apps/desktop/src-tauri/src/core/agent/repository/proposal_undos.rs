//! Independent undo baseline and journal. Never reinterpret an original apply.
use super::{change_sets, execution_workspaces::Error, proposal_applications as applications};
use applications::{ApplicationScope, ApplicationStatus, ApplicationTarget, StoredApplication};
use research_agent_core::workspace::{
    BaselineFile, FileChange, FileChangeKind, SourceVersion, WorkspaceFile, WorkspaceIdentity,
    WorkspaceManifest,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

#[path = "proposal_undos/editor_restoration.rs"]
mod editor_restoration;
#[path = "proposal_undos/progress.rs"]
mod progress;
pub use editor_restoration::{
    acknowledge_editor_restored_in, begin_editor_restoration_in, read_editor_restoration_in,
};
#[path = "proposal_undos/rollback.rs"]
mod rollback;
#[path = "proposal_undos/startup.rs"]
mod startup;
#[path = "proposal_undos/targets.rs"]
mod targets;
pub use progress::{
    acknowledge_in, cancel_pending_in, dispatch_in, finish_in, require_recovery_in, resume_in,
    start_in,
};
pub use rollback::{acknowledge_disk_restored_in, finish_rollback_in};
pub use startup::recover_on_startup;
pub(crate) use targets::rebind_pending_editor_targets_in;

pub struct StoredUndo {
    pub original_operation: Uuid,
    pub application: StoredApplication,
    pub changes: Vec<FileChange>,
}

/// Transaction-scoped lookup. SQLite insertion order avoids wall-clock ties or
/// clock rollback; admission permits only one unresolved undo per proposal.
pub fn find_latest_in(
    db: &Connection,
    original: ApplicationScope,
) -> Result<Option<StoredUndo>, Error> {
    let applied = applications::read_in(
        db,
        original.conversation,
        original.manuscript,
        original.proposal,
        original.operation,
    )?;
    if applied.status != ApplicationStatus::Applied {
        return Err(Error::Conflict);
    }
    let operation: Option<String> = db.query_row(
        "select operation_id from agent_proposal_undos where original_operation_id=?1 order by rowid desc limit 1",
        [original.operation.to_string()], |r| r.get(0),
    ).optional()?;
    match operation {
        None => Ok(None),
        Some(value) => read_in(
            db,
            original,
            Uuid::parse_str(&value).map_err(|_| Error::Conflict)?,
        ),
    }
}

/// Caller holds a consistent transaction. Existing means historical state, never
/// permission to repeat preflight or dispatch an editor/file write.
pub fn read_in(
    db: &Connection,
    original: ApplicationScope,
    operation: Uuid,
) -> Result<Option<StoredUndo>, Error> {
    if operation.is_nil() || operation == original.operation {
        return Err(Error::Conflict);
    }
    let applied = applications::read_in(
        db,
        original.conversation,
        original.manuscript,
        original.proposal,
        original.operation,
    )?;
    if applied.status != ApplicationStatus::Applied {
        return Err(Error::Conflict);
    }
    let origin: Option<String> = db
        .query_row(
            "select original_operation_id from agent_proposal_undos where operation_id=?1",
            [operation.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(origin) = origin else {
        let occupied: bool = db.query_row(
            "select exists(select 1 from agent_proposal_applications where id=?1)",
            [operation.to_string()],
            |r| r.get(0),
        )?;
        return if occupied {
            Err(Error::Conflict)
        } else {
            Ok(None)
        };
    };
    if origin != original.operation.to_string() {
        return Err(Error::Conflict);
    }
    let valid: bool = db.query_row("select exists(select 1 from agent_proposal_applications where id=?1 and change_set_id=?2 and direction='undo')", params![operation.to_string(), original.proposal.to_string()], |r| r.get(0))?;
    if !valid {
        return Err(Error::Conflict);
    }
    let proposal = change_sets::read_file_operation_in(
        db,
        original.conversation,
        original.manuscript,
        original.proposal,
    )?;
    let record = applications::read_record_in(db, original.proposal, operation)?;
    let mut query = db.prepare("select relative_path,source_json from agent_proposal_undo_sources where operation_id=?1 order by relative_path limit 20001")?;
    let rows = query
        .query_map([operation.to_string()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != proposal.changes.len() || record.files.len() != rows.len() {
        return Err(Error::Conflict);
    }
    let mut changes = Vec::with_capacity(rows.len());
    for (((path, json), forward), file) in
        rows.into_iter().zip(&proposal.changes).zip(&record.files)
    {
        if path != forward.path || file.path != path {
            return Err(Error::Conflict);
        }
        let source: Option<SourceVersion> = json
            .map(|value| {
                if value.len() > 16 * 1024 {
                    return Err(Error::Conflict);
                }
                serde_json::from_str(&value).map_err(|_| Error::Conflict)
            })
            .transpose()?;
        // Sources retain the frozen preparation provenance. A fully pending
        // inverse can acquire a new live owner through whole-plan preflight;
        // per-file dispatch and ACK use that target's current CAS identity.
        let expected = source.as_ref().and_then(|s| s.editor.as_ref());
        match (&file.target, expected) {
            (ApplicationTarget::Disk, None) => {}
            (ApplicationTarget::Editor { .. }, Some(_)) => {}
            _ => return Err(Error::Conflict),
        }
        changes.push(FileChange {
            path,
            before: forward.after.clone(),
            after: forward.before.clone(),
            kind: reverse_kind(forward.kind),
            source,
        });
    }
    validate(&proposal, &changes)?;
    Ok(Some(StoredUndo {
        original_operation: original.operation,
        application: record,
        changes,
    }))
}

/// Caller has preflighted current sources in this same immediate transaction and
/// owns complete editor coverage. Persist every target before returning prepared.
pub fn begin_in(
    tx: &mut Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
    changes: &[FileChange],
) -> Result<StoredUndo, Error> {
    if let Some(saved) = read_in(tx, original, operation)? {
        return Ok(saved);
    }
    let applied = applications::read_in(
        tx,
        original.conversation,
        original.manuscript,
        original.proposal,
        original.operation,
    )?;
    let proposal = change_sets::read_file_operation_in(
        tx,
        original.conversation,
        original.manuscript,
        original.proposal,
    )?;
    if applied.status != ApplicationStatus::Applied
        || proposal.status != change_sets::ChangeSetStatus::Applied
    {
        return Err(Error::Conflict);
    }
    validate(&proposal, changes)?;
    let sp = tx.savepoint()?;
    sp.execute(
        "insert into agent_proposal_applications(id,change_set_id,direction) values(?1,?2,'undo')",
        params![operation.to_string(), original.proposal.to_string()],
    )?;
    sp.execute(
        "insert into agent_proposal_undos(operation_id,original_operation_id) values(?1,?2)",
        params![operation.to_string(), original.operation.to_string()],
    )?;
    for change in changes {
        let editor = change.source.as_ref().and_then(|s| s.editor.as_ref());
        sp.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind,expected_editor_session,expected_editor_revision) values(?1,?2,?3,?4,?5,?6)", params![operation.to_string(), original.proposal.to_string(),change.path,if editor.is_some(){"editor"}else{"disk"},editor.map(|e|e.session_id.to_string()),editor.map(|e|e.revision.to_string())])?;
        let source = change
            .source
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| Error::Conflict)?;
        sp.execute("insert into agent_proposal_undo_sources(operation_id,relative_path,source_json) values(?1,?2,?3)", params![operation.to_string(),change.path,source])?;
    }
    let saved = read_in(&sp, original, operation)?.ok_or(Error::Conflict)?;
    sp.commit()?;
    Ok(saved)
}

fn reverse_kind(kind: FileChangeKind) -> FileChangeKind {
    match kind {
        FileChangeKind::Added => FileChangeKind::Deleted,
        FileChangeKind::Deleted => FileChangeKind::Added,
        FileChangeKind::Modified => FileChangeKind::Modified,
    }
}

fn validate(proposal: &change_sets::StoredChangeSet, changes: &[FileChange]) -> Result<(), Error> {
    if changes.len() != proposal.changes.len() || changes.is_empty() {
        return Err(Error::Conflict);
    }
    let mut files = Vec::new();
    for (reverse, forward) in changes.iter().zip(&proposal.changes) {
        if reverse.path != forward.path
            || reverse.before != forward.after
            || reverse.after != forward.before
            || reverse.kind != reverse_kind(forward.kind)
            || reverse.before.is_some() != reverse.source.is_some()
            || (forward.source.as_ref().is_some_and(|s| s.editor.is_some())
                && reverse.source.as_ref().is_none_or(|s| s.editor.is_none()))
        {
            return Err(Error::Conflict);
        }
        if let (Some(version), Some(source)) = (&reverse.before, &reverse.source) {
            files.push(BaselineFile {
                file: WorkspaceFile {
                    path: reverse.path.clone(),
                    version: version.clone(),
                },
                source: source.clone(),
            });
        }
    }
    WorkspaceManifest {
        schema_version: 1,
        identity: WorkspaceIdentity {
            conversation_id: proposal.binding.conversation_id,
            turn_id: proposal.binding.turn_id,
            lease_id: proposal.binding.lease_id,
            environment_id: format!("manuscript:{}", proposal.binding.manuscript_id),
        },
        files,
    }
    .validate()
    .map_err(|_| Error::Conflict)
}
