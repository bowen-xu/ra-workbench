//! Proposal provenance is independent of the disposable execution binding.
use super::*;

pub(super) fn read(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
) -> Result<(Binding, WorkspaceManifest), Error> {
    let result = read_stored(db, conversation, manuscript, proposal)?;
    execution_workspaces::visible(db, &result.0)?;
    Ok(result)
}

/// Existing file operations belong to the active manuscript. Frozen immutable
/// provenance and an admitted apply journal prove origin without a live chat
/// binding. This does not authorize execution or a new proposal application.
pub(super) fn read_file_operation(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
) -> Result<(Binding, WorkspaceManifest), Error> {
    let result = read_stored(db, conversation, manuscript, proposal)?;
    let b = &result.0;
    let allowed: bool = db.query_row(
        "select exists(select 1 from agent_change_sets c
         join manuscripts m on m.id=c.manuscript_id and m.status='active'
         where c.conversation_id=?1 and c.request_id=?2 and c.turn_id=?3 and c.context_snapshot_id=?4
         and c.manuscript_id=?5 and c.id=?6
         and exists(select 1 from agent_proposal_applications a where a.change_set_id=c.id and a.direction='apply'))",
        params![b.conversation_id.to_string(), b.request_id.to_string(), b.turn_id.to_string(),
            b.context_snapshot_id.to_string(), b.manuscript_id.to_string(), proposal.to_string()],
        |r| r.get(0),
    )?;
    if !allowed {
        return Err(Error::Denied);
    }
    Ok(result)
}

fn read_stored(
    db: &Connection,
    conversation: Uuid,
    manuscript: Uuid,
    proposal: Uuid,
) -> Result<(Binding, WorkspaceManifest), Error> {
    let (binding, json): (Binding, String) = db.query_row(
        "select request_id,turn_id,context_snapshot_id,lease_id,manifest_hash,baseline_json from agent_change_sets where id=?1 and conversation_id=?2 and manuscript_id=?3 and schema_version=1",
        params![proposal.to_string(), conversation.to_string(), manuscript.to_string()],
        |r| {
            let id = |column| Uuid::parse_str(&r.get::<_, String>(column)?)
                .map_err(|_| rusqlite::Error::InvalidQuery);
            Ok((Binding { conversation_id: conversation, request_id: id(0)?, turn_id: id(1)?,
                context_snapshot_id: id(2)?, manuscript_id: manuscript, lease_id: id(3)?, manifest_hash: r.get(4)? }, r.get(5)?))
        },
    ).optional()?.ok_or(Error::Denied)?;
    if json.len() > 32 * 1024 * 1024 {
        return Err(Error::Conflict);
    }
    let manifest: WorkspaceManifest = serde_json::from_str(&json).map_err(|_| Error::Conflict)?;
    if execution_workspaces::encoded(&manifest)?.0 != binding.manifest_hash
        || manifest.identity.conversation_id != binding.conversation_id
        || manifest.identity.turn_id != binding.turn_id
        || manifest.identity.lease_id != binding.lease_id
    {
        return Err(Error::Conflict);
    }
    Ok((binding, manifest))
}
