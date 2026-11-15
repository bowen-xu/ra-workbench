//! Freeze original workspace metadata; discard stays in its existing file owner.
use super::*;
#[derive(PartialEq, Eq)]
pub(super) struct WorkspaceRow {
    request: String,
    turn: String,
    context: String,
    manuscript: String,
    lease: String,
    manifest_hash: String,
    manifest: String,
    policy: String,
    state: String,
}
pub(super) fn workspaces(
    db: &Connection,
    conversation: ConversationId,
) -> Result<Vec<WorkspaceRow>, DeletionSqlError> {
    let mut statement=db.prepare("select request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json,policy,state from agent_execution_workspaces where conversation_id=?1 order by request_id")?;
    Ok(statement
        .query_map([conversation.as_uuid().to_string()], |row| {
            Ok(WorkspaceRow {
                request: row.get(0)?,
                turn: row.get(1)?,
                context: row.get(2)?,
                manuscript: row.get(3)?,
                lease: row.get(4)?,
                manifest_hash: row.get(5)?,
                manifest: row.get(6)?,
                policy: row.get(7)?,
                state: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}
pub(super) fn purge_workspace_files(
    rows: &[WorkspaceRow],
    request: &Request,
    root: Option<std::path::PathBuf>,
) -> Result<(), Error> {
    if rows.is_empty() {
        return Ok(());
    }
    use research_agent_core::conversation::CatalogDataFailureKind as K;
    let fail = |kind, cause| {
        workspace_failure(request.request_id, kind, "delete-workspace-discard", cause)
    };
    let Some(root) = root else {
        return Err(fail(K::Unavailable, "workspace_directory_unavailable"));
    };
    #[cfg(unix)]
    {
        use crate::agent::workspace::{LeaseError, WorkspaceStore};
        use research_agent_core::workspace::WorkspaceIdentity;
        // Decode all identities before deleting the first file. Corrupt later
        // metadata must not turn an earlier successful cleanup into a refusal.
        let identities = rows
            .iter()
            .map(|row| {
                Ok(WorkspaceIdentity {
                    conversation_id: *request.conversation_id.as_uuid(),
                    turn_id: Uuid::parse_str(&row.turn).map_err(|_| CatalogError::Corrupt)?,
                    environment_id: format!("manuscript:{}", row.manuscript),
                    lease_id: Uuid::parse_str(&row.lease).map_err(|_| CatalogError::Corrupt)?,
                })
            })
            .collect::<Result<Vec<_>, CatalogError>>()?;
        let canonical = root.canonicalize().map_err(|cause| {
            fail(
                K::Unavailable,
                match cause.kind() {
                    std::io::ErrorKind::NotFound => "workspace_root_not_found",
                    std::io::ErrorKind::PermissionDenied => "workspace_root_permission_denied",
                    _ => "workspace_root_unavailable",
                },
            )
        })?;
        let store = WorkspaceStore::open(&canonical, |_| Ok(()))
            .map_err(|_| fail(K::Unavailable, "workspace_store_unavailable"))?;
        let mut cleaned = false;
        for identity in identities {
            if let Err(cause) = store.discard(&identity) {
                if matches!(cause, LeaseError::Busy) && !cleaned {
                    return Err(CatalogError::Busy.into());
                }
                return Err(fail(
                    if cleaned {
                        K::Partial
                    } else {
                        K::ExternalUnknown
                    },
                    match cause {
                        LeaseError::Busy => "workspace_discard_partial_busy",
                        LeaseError::Conflict => "workspace_discard_conflict",
                        LeaseError::Incomplete => "workspace_discard_incomplete",
                        _ => "workspace_discard_unconfirmed",
                    },
                ));
            }
            cleaned = true;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        return Err(fail(
            K::Unavailable,
            "workspace_discard_platform_unavailable",
        ));
    }
    Ok(())
}

/// Hold every real workspace owner until the batch metadata transaction ends.
/// A completed discard receipt is already fenced by WorkspaceStore itself.
#[cfg(unix)]
pub(super) fn hold_workspace_files(
    rows: &[WorkspaceRow],
    request: &Request,
    root: Option<std::path::PathBuf>,
) -> Result<Vec<crate::agent::workspace::WorkspaceLease>, Error> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    use crate::agent::workspace::{DiscardState, LeaseError, WorkspaceStore};
    use research_agent_core::workspace::WorkspaceIdentity;
    let root = root
        .ok_or(CatalogError::Unavailable)?
        .canonicalize()
        .map_err(|_| CatalogError::Unavailable)?;
    let store = WorkspaceStore::open(&root, |_| Ok(())).map_err(|_| CatalogError::Unavailable)?;
    let mut held = Vec::new();
    for row in rows {
        let identity = WorkspaceIdentity {
            conversation_id: *request.conversation_id.as_uuid(),
            turn_id: Uuid::parse_str(&row.turn).map_err(|_| CatalogError::Corrupt)?,
            environment_id: format!("manuscript:{}", row.manuscript),
            lease_id: Uuid::parse_str(&row.lease).map_err(|_| CatalogError::Corrupt)?,
        };
        if store
            .discard_state(&identity)
            .map_err(|_| CatalogError::Unavailable)?
            == Some(DiscardState::Complete)
        {
            // A receipt alone cannot authorize a replacement directory. The
            // existing discard path checks it is still absent before replay.
            store.discard(&identity).map_err(|cause| match cause {
                LeaseError::Busy | LeaseError::Incomplete => CatalogError::Busy,
                LeaseError::Conflict | LeaseError::Invalid => CatalogError::Conflict,
                _ => CatalogError::Unavailable,
            })?;
            continue;
        }
        let lease = store.restore(&identity).map_err(|cause| match cause {
            LeaseError::Busy | LeaseError::Incomplete => CatalogError::Busy,
            LeaseError::Conflict | LeaseError::Invalid => CatalogError::Conflict,
            _ => CatalogError::Unavailable,
        })?;
        held.push(lease);
    }
    Ok(held)
}
