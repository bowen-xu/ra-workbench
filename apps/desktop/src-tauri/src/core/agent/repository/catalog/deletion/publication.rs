//! Pure SQL body; no file/process effects and no transaction ownership.
use super::*;
pub(crate) fn delete_in(
    tx: &rusqlite::Transaction<'_>,
    prepared: &PreparedDeletion,
) -> Result<(), DeletionSqlError> {
    let request = &prepared.request;
    let hash = &prepared.hash;
    if workspaces(tx, request.conversation_id)? != prepared.workspaces {
        return Err(CatalogError::Conflict.into());
    }
    check_state_sql(tx, request)?;
    if request.force {
        tx.execute(
            "insert into agent_force_delete_markers(conversation_id,request_id) values (?1,?2)",
            params![
                request.conversation_id.as_uuid().to_string(),
                request.request_id.to_string()
            ],
        )?;
    }
    if replay_sql(tx, request, hash)?.is_some() {
        // A legacy tombstone cannot be certified retroactively.
        return Err(CatalogError::Conflict.into());
    }
    let id = request.conversation_id.as_uuid().to_string();
    if request.force {
        purge_retained_rows(tx, request.conversation_id.as_uuid())?;
    }
    tx.execute("insert into agent_conversation_deletions(request_id,conversation_id,request_hash) values (?1,?2,?3)",
        params![request.request_id.to_string(),id,hash])?;
    if tx.execute("delete from agent_conversations where id=?1", [&id])? != 1 {
        return Err(CatalogError::Conflict.into());
    }
    if request.force {
        tx.execute(
            "delete from agent_force_delete_markers where conversation_id=?1 and request_id=?2",
            params![
                request.conversation_id.as_uuid().to_string(),
                request.request_id.to_string()
            ],
        )?;
    }
    Ok(())
}

/// Remove the rows whose normal foreign-key retention rules protect history.
/// Workspace directories are removed before this transaction by the catalog
/// adapter; the marker only relaxes the two discard journal triggers during
/// this atomic metadata purge.
fn purge_retained_rows(
    tx: &rusqlite::Transaction<'_>,
    conversation: &Uuid,
) -> Result<(), rusqlite::Error> {
    let id = conversation.to_string();
    tx.execute(
        "delete from agent_fork_turn_sources where conversation_id=?1 or source_conversation_id=?1",
        [&id],
    )?;
    tx.execute(
        "delete from agent_conversation_forks where child_id=?1 or parent_id=?1",
        [&id],
    )?;
    tx.execute(
        "delete from agent_subtasks where child_conversation_id=?1 or parent_conversation_id=?1",
        [&id],
    )?;
    tx.execute(
        "delete from agent_execution_workspaces where conversation_id=?1",
        [&id],
    )?;
    tx.execute(
        "delete from agent_workspace_discards where conversation_id=?1",
        [&id],
    )?;
    Ok(())
}

/// Recheck confirmation while holding the same admission/SQL lock before files
/// and again in the publication transaction. This does not authorize a task.
pub(super) fn check_state_sql(db: &Connection, request: &Request) -> Result<(), DeletionSqlError> {
    let id = request.conversation_id.as_uuid().to_string();
    let row = db.query_row("select id,title,archived,durable_count,created_at,updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=c.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=c.id)) from agent_conversations c where id=?1 and (?2 is null or exists
        (select 1 from agent_conversation_bindings b where b.conversation_id=c.id and b.resource_kind=?2 and b.resource_id=?3))",
        params![id,request.binding.as_ref().map(|b|binding_kind(b.kind)),request.binding.as_ref().map(|b|&b.resource_id)], read_row).optional()?.ok_or(CatalogError::NotFound)?;
    check_summary(&decode(row)?, request)?;
    for (query, reason) in [
        (
            "select exists(select 1 from agent_conversation_forks where parent_id=?1) or exists(select 1 from agent_fork_turn_sources where source_conversation_id=?1)",
            Retention::BranchHistory,
        ),
        (
            "select exists(select 1 from agent_execution_workspaces where conversation_id=?1)",
            Retention::ExecutionWorkspace,
        ),
    ] {
        let retained: bool = db.query_row(query, [&id], |row| row.get(0))?;
        if retained && !request.force {
            return Err(Error::Retained(reason).into());
        }
    }
    Ok(())
}
