//! Frozen metadata lookup independent of the execution directory lifecycle.
use super::*;
use research_agent_core::workspace::{FileChangeKind, FileVersion, SourceVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetStatus {
    Pending,
    Rejected,
    Applying,
    Applied,
    Conflict,
    RecoveryRequired,
    Undone,
}
impl ChangeSetStatus {
    fn parse(value: &str) -> Result<Self, Error> {
        Ok(match value {
            "pending" => Self::Pending,
            "rejected" => Self::Rejected,
            "applying" => Self::Applying,
            "applied" => Self::Applied,
            "conflict" => Self::Conflict,
            "recovery_required" => Self::RecoveryRequired,
            "undone" => Self::Undone,
            _ => return Err(Error::Conflict),
        })
    }
}
pub struct StoredChangeSet {
    pub id: Uuid,
    pub binding: Binding,
    pub status: ChangeSetStatus,
    pub changes: Vec<FileChange>,
}

/// Caller holds a read transaction. No raw content or execution authority escapes.
pub fn read_in(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    id: Uuid,
) -> Result<StoredChangeSet, Error> {
    read_validated(db, conversation, manuscript, id, baseline::read)
}

/// Frozen origin for an already admitted file operation; callers must also
/// validate their exact operation identity and journal before any transition.
pub(crate) fn read_file_operation_in(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    id: Uuid,
) -> Result<StoredChangeSet, Error> {
    read_validated(
        db,
        conversation,
        manuscript,
        id,
        baseline::read_file_operation,
    )
}

fn read_validated(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    id: Uuid,
    read_baseline: impl FnOnce(
        &Connection,
        Uuid,
        Uuid,
        Uuid,
    ) -> Result<(Binding, WorkspaceManifest), Error>,
) -> Result<StoredChangeSet, Error> {
    if conversation.is_nil() || manuscript.is_nil() || id.is_nil() {
        return Err(Error::Conflict);
    }
    let (hash, status): (String,String) = db.query_row(
        "select inventory_hash,status from agent_change_sets where id=?1 and conversation_id=?2 and manuscript_id=?3",
        params![id.to_string(),conversation.to_string(),manuscript.to_string()],
        |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional()?.ok_or(Error::Denied)?;
    let (binding, baseline) = read_baseline(db, conversation, manuscript, id)?;
    let mut statement = db.prepare("select f.relative_path,f.before_sha256,b.byte_size,f.before_executable,f.after_sha256,a.byte_size,f.after_executable,f.source_json from agent_change_set_files f left join content_objects b on b.sha256=f.before_sha256 left join content_objects a on a.sha256=f.after_sha256 where f.change_set_id=?1 order by f.relative_path limit 20001")?;
    let rows = statement
        .query_map([id.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                side(r, 1)?,
                side(r, 4)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.is_empty() || rows.len() > 20_000 {
        return Err(Error::Conflict);
    }
    let mut changes = Vec::with_capacity(rows.len());
    for (path, before, after, source) in rows {
        let source: Option<SourceVersion> = source
            .map(|json| {
                if json.len() > 16 * 1024 {
                    return Err(Error::Conflict);
                }
                serde_json::from_str(&json).map_err(|_| Error::Conflict)
            })
            .transpose()?;
        let kind = match (&before, &after) {
            (None, Some(_)) => FileChangeKind::Added,
            (Some(_), None) => FileChangeKind::Deleted,
            (Some(_), Some(_)) => FileChangeKind::Modified,
            _ => return Err(Error::Conflict),
        };
        changes.push(FileChange {
            path,
            before,
            after,
            source,
            kind,
        });
    }
    validate_inventory(&baseline, &changes)?;
    let actual = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&changes).map_err(|_| Error::Conflict)?)
    );
    if actual != hash {
        return Err(Error::Conflict);
    }
    verify_existing(db, &id.to_string(), &binding, &changes)?;
    Ok(StoredChangeSet {
        id,
        binding,
        status: ChangeSetStatus::parse(&status)?,
        changes,
    })
}
fn side(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<Option<FileVersion>> {
    let hash: Option<String> = row.get(start)?;
    let size: Option<u64> = row.get(start + 1)?;
    let mode: Option<bool> = row.get(start + 2)?;
    match (hash, size, mode) {
        (None, None, None) => Ok(None),
        (Some(sha256), Some(byte_size), Some(executable)) => Ok(Some(FileVersion {
            sha256,
            byte_size,
            executable,
        })),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
