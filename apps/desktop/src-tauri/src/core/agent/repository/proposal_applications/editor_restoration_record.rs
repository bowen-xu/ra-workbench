//! Shared restoration journal I/O after the caller validates its operation baseline.
use super::*;
use research_agent_core::workspace::EditorVersion;

pub(crate) fn read_editor_restoration_record_in(
    db: &Connection,
    saved: &StoredApplication,
    path: &str,
) -> Result<Option<EditorRestoration>, Error> {
    if !saved
        .files
        .iter()
        .any(|file| file.path == path && matches!(file.target, ApplicationTarget::Editor { .. }))
    {
        return Err(Error::Conflict);
    }
    let row: Option<(String, String, String, Option<String>)> = db.query_row(
        "select restore_id,editor_session,expected_revision,result_revision from agent_editor_restorations where application_id=?1 and relative_path=?2",
        params![saved.operation_id.to_string(),path], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional()?;
    let parse = |value: &str| {
        Uuid::parse_str(value)
            .ok()
            .filter(|id| !id.is_nil())
            .ok_or(Error::Conflict)
    };
    row.map(|(id, session, revision, result)| {
        Ok(EditorRestoration {
            id: parse(&id)?,
            expected: EditorVersion {
                session_id: parse(&session)?,
                revision: parse(&revision)?,
            },
            result_revision: result.as_deref().map(parse).transpose()?,
        })
    })
    .transpose()
}

pub(crate) fn begin_editor_restoration_record_in(
    tx: &Transaction<'_>,
    saved: &StoredApplication,
    path: &str,
    restore_id: Uuid,
    expected: &EditorVersion,
) -> Result<EditorRestorationAdmission, Error> {
    if restore_id.is_nil()
        || restore_id == saved.operation_id
        || expected.session_id.is_nil()
        || expected.revision.is_nil()
    {
        return Err(Error::Conflict);
    }
    if let Some(saved) = read_editor_restoration_record_in(tx, saved, path)? {
        if saved.id != restore_id || &saved.expected != expected {
            return Err(Error::Conflict);
        }
        return Ok(EditorRestorationAdmission::Existing(saved));
    }
    tx.execute("insert into agent_editor_restorations(application_id,relative_path,restore_id,editor_session,expected_revision) values(?1,?2,?3,?4,?5)",
        params![saved.operation_id.to_string(),path,restore_id.to_string(),expected.session_id.to_string(),expected.revision.to_string()])?;
    Ok(EditorRestorationAdmission::Created(EditorRestoration {
        id: restore_id,
        expected: expected.clone(),
        result_revision: None,
    }))
}

/// Caller validates receipt, target phase and business state; savepoint keeps
/// receipt and phase atomic even if an outer transaction catches an error.
pub(crate) fn commit_editor_restoration_record_in(
    tx: &mut Transaction<'_>,
    operation: Uuid,
    path: &str,
    revision: Uuid,
) -> Result<(), Error> {
    let sp = tx.savepoint()?;
    if sp.execute("update agent_editor_restorations set result_revision=?3 where application_id=?1 and relative_path=?2 and result_revision is null",
        params![operation.to_string(),path,revision.to_string()])? != 1
        || sp.execute("update agent_proposal_application_files set phase='restored',updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where application_id=?1 and relative_path=?2 and phase='acknowledged'",
        params![operation.to_string(),path])? != 1 {
        return Err(Error::Conflict);
    }
    sp.commit()?;
    Ok(())
}
