//! Read committed application progress without acquiring execution authority.
use super::*;
use research_agent_core::workspace::EditorVersion;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationStatus {
    Prepared,
    Applying,
    Applied,
    RecoveryRequired,
    RolledBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationPhase {
    Pending,
    Dispatched,
    Acknowledged,
    Restored,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationTarget {
    Disk,
    Editor {
        expected: EditorVersion,
        result_revision: Option<Uuid>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationFile {
    pub path: String,
    pub target: ApplicationTarget,
    pub phase: ApplicationPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredApplication {
    pub operation_id: Uuid,
    pub proposal_id: Uuid,
    pub status: ApplicationStatus,
    pub files: Vec<ApplicationFile>,
}

/// Caller owns a read transaction. Only apply-direction records are supported;
/// undo needs its own baseline validation. No bytes or dispatch tokens escape.
pub fn read_in(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
    operation: Uuid,
) -> Result<StoredApplication, Error> {
    // Revalidate the complete frozen plan and business scope, not just the ID.
    if !exists_in(db, conversation, manuscript, proposal, operation)? {
        return Err(Error::Denied);
    }
    read_record_in(db, proposal, operation)
}

/// Only after the caller validates direction, scope and the entire frozen baseline.
pub(crate) fn read_record_in(
    db: &Connection,
    proposal: Uuid,
    operation: Uuid,
) -> Result<StoredApplication, Error> {
    let status: String = db.query_row(
        "select status from agent_proposal_applications where id=?1",
        [operation.to_string()],
        |r| r.get(0),
    )?;
    let status = match status.as_str() {
        "prepared" => ApplicationStatus::Prepared,
        "applying" => ApplicationStatus::Applying,
        "applied" => ApplicationStatus::Applied,
        "recovery_required" => ApplicationStatus::RecoveryRequired,
        "rolled_back" => ApplicationStatus::RolledBack,
        _ => return Err(Error::Conflict),
    };
    let mut statement = db.prepare(
        "select relative_path,target_kind,expected_editor_session,expected_editor_revision,phase,result_editor_revision
         from agent_proposal_application_files where application_id=?1 order by relative_path limit 20001",
    )?;
    let rows = statement.query_map([operation.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut files = Vec::new();
    for row in rows {
        let (path, kind, session, revision, phase, result) = row?;
        let phase = match phase.as_str() {
            "pending" => ApplicationPhase::Pending,
            "dispatched" => ApplicationPhase::Dispatched,
            "acknowledged" => ApplicationPhase::Acknowledged,
            "restored" => ApplicationPhase::Restored,
            "conflict" => ApplicationPhase::Conflict,
            _ => return Err(Error::Conflict),
        };
        let target = match (kind.as_str(), session, revision, result) {
            ("disk", None, None, None) => ApplicationTarget::Disk,
            ("editor", Some(session), Some(revision), result) => {
                let result_revision = result.as_deref().map(uuid).transpose()?;
                if phase == ApplicationPhase::Acknowledged && result_revision.is_none() {
                    return Err(Error::Conflict);
                }
                if phase == ApplicationPhase::Pending && result_revision.is_some() {
                    return Err(Error::Conflict);
                }
                ApplicationTarget::Editor {
                    expected: EditorVersion {
                        session_id: uuid(&session)?,
                        revision: uuid(&revision)?,
                    },
                    result_revision,
                }
            }
            _ => return Err(Error::Conflict),
        };
        if (status == ApplicationStatus::Applied && phase != ApplicationPhase::Acknowledged)
            || (status == ApplicationStatus::RolledBack
                && !matches!(
                    phase,
                    ApplicationPhase::Pending | ApplicationPhase::Restored
                ))
            || (status == ApplicationStatus::Prepared && phase != ApplicationPhase::Pending)
        {
            return Err(Error::Conflict);
        }
        files.push(ApplicationFile {
            path,
            target,
            phase,
        });
    }
    if files.is_empty() || files.len() > 20_000 {
        return Err(Error::Conflict);
    }
    Ok(StoredApplication {
        operation_id: operation,
        proposal_id: proposal,
        status,
        files,
    })
}

fn uuid(value: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(value)
        .ok()
        .filter(|id| !id.is_nil() && id.to_string() == value)
        .ok_or(Error::Conflict)
}

/// Recover the application identity from its frozen proposal, including after a
/// renderer restart. A missing journal is distinct from an inaccessible proposal.
pub fn find_in(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
) -> Result<Option<StoredApplication>, Error> {
    let mut query = db.prepare("select id from agent_proposal_applications where change_set_id=?1 and direction='apply' limit 2")?;
    let operations = query
        .query_map([proposal.to_string()], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if operations.len() > 1 {
        return Err(Error::Conflict);
    }
    if operations.is_empty() {
        // No admitted file operation: retain current conversation visibility.
        change_sets::read_in(db, conversation, manuscript, proposal)?;
    }
    operations
        .first()
        .map(|value| read_in(db, conversation, manuscript, proposal, uuid(value)?))
        .transpose()
}
