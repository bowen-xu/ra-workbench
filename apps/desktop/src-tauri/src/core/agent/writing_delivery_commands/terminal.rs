//! Resolve terminal delivery identity without guessing from proposal timestamps.
use super::super::writing_workspace_commands::Confirmation;
use super::repository::{id, storage};
use super::*;

pub(super) fn latest_binding(
    db: &Connection,
    request: &Acknowledge,
) -> Result<Confirmation, Error> {
    let scope = &request.scope;
    let has_generations: bool = db.query_row("select exists(select 1 from sqlite_master where type='table' and name='writing_agent_workspace_generations')", [], |row| row.get(0)).map_err(storage)?;
    let generation: Option<(String, String, String, String, String)> = if has_generations {
        db.query_row("select turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash from writing_agent_workspace_generations where conversation_id=?1 and request_id=?2 order by generation desc limit 1",
            params![scope.conversation_id.to_string(),scope.request_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional().map_err(storage)?
    } else {
        None
    };
    let (turn, context, manuscript, lease, manifest_hash) = match generation {
        Some(value) => value,
        None => db.query_row("select turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash from agent_execution_workspaces where conversation_id=?1 and request_id=?2",
            params![scope.conversation_id.to_string(),scope.request_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional().map_err(storage)?.ok_or(Error::Conflict)?,
    };
    if id(&turn)? != request.turn_id
        || id(&context)? != scope.context_snapshot_id
        || id(&manuscript)? != scope.manuscript_id
    {
        return Err(Error::Conflict);
    }
    let binding = Confirmation {
        scope: scope.clone(),
        turn_id: request.turn_id,
        lease_id: id(&lease)?,
        manifest_hash,
    };
    binding.binding()?;
    Ok(binding)
}

#[cfg(target_os = "macos")]
pub(super) fn latest_for_scope(db: &Connection, scope: &Scope) -> Result<Confirmation, Error> {
    scope.validate()?;
    let turn: String = db.query_row("select turn_id from agent_execution_workspaces where conversation_id=?1 and request_id=?2 and manuscript_id=?3 and context_snapshot_id=?4",
        params![scope.conversation_id.to_string(),scope.request_id.to_string(),scope.manuscript_id.to_string(),scope.context_snapshot_id.to_string()], |row| row.get(0)).optional().map_err(storage)?.ok_or(Error::Conflict)?;
    latest_binding(
        db,
        &Acknowledge {
            scope: scope.clone(),
            turn_id: id(&turn)?,
        },
    )
}
