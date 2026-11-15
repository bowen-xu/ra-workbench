//! Read-only discovery of admitted file operations, independent of chat lifecycle.
use super::*;
#[cfg(target_os = "macos")]
use crate::agent::repository::{change_sets, proposal_applications as applications};
#[cfg(target_os = "macos")]
use rusqlite::{OptionalExtension, params};
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListRequest {
    pub manuscript_id: Uuid,
    pub after: Option<u64>,
    pub limit: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u64,
    pub application: Response,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub schema_version: u8,
    pub manuscript_id: Uuid,
    pub items: Vec<Entry>,
    pub next_cursor: Option<u64>,
}

#[cfg(target_os = "macos")]
pub(crate) async fn list(
    store: &crate::storage::AppStore,
    request: ListRequest,
) -> Result<Page, Error> {
    if request.manuscript_id.is_nil()
        || !(1..=64).contains(&request.limit)
        || request
            .after
            .is_some_and(|id| id == 0 || id > 9_007_199_254_740_991)
    {
        return Err(Error::InvalidInput);
    }
    if store.writing_workspace_source().is_none()
        && let Some(binding) = store.native_writing_context()
    {
        return binding
            .list_pending_applications(store.clone(), request)
            .await;
    }
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        list_with_checkpoints(&store, &request, |db, scope, path| {
            use crate::ai::structured_resource_validation::Flow;
            let read = store.writing_checkpoint_source().ok_or_else(|| Flow::original(Error::Conflict))?;
            read(db, scope.manuscript, path).map_err(|_| Flow::original(Error::Conflict))
        }).map_err(crate::ai::structured_resource_validation::Flow::<Error,std::convert::Infallible>::into_static)
    }).await.map_err(|_| Error::Unavailable)?
}

#[cfg(target_os = "macos")]
pub(crate) fn list_with_checkpoints<P>(
    store: &crate::storage::AppStore,
    request: &ListRequest,
    mut read_checkpoint: impl FnMut(
        &rusqlite::Connection,
        crate::agent::repository::proposal_applications::ApplicationScope,
        &str,
    ) -> Result<
        Option<crate::agent::writing_checkpoint_contracts::CheckpointRecord>,
        crate::ai::structured_resource_validation::Flow<Error, P>,
    >,
) -> Result<Page, crate::ai::structured_resource_validation::Flow<Error, P>> {
    use crate::ai::structured_resource_validation::Flow;
    let mut db = store
        .connection()
        .map_err(|_| Flow::original(Error::Unavailable))?;
    let tx = db
        .transaction()
        .map_err(|_| Flow::original(Error::Unavailable))?;
    let result = (|| {
        let active: Option<bool> = tx
            .query_row(
                "select status='active' from manuscripts where id=?1",
                [request.manuscript_id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Flow::original(Error::Unavailable))?;
        if active != Some(true) {
            return Err(Flow::original(Error::Denied));
        }
        // The admitted application and frozen inventory own the operation. No
        // conversation, binding, execution workspace or history join is needed.
        let mut query = tx.prepare(
    "select a.rowid,a.id,c.id,c.conversation_id
             from agent_proposal_applications a
             join agent_change_sets c on c.id=a.change_set_id
             where c.manuscript_id=?1 and a.direction='apply' and a.rowid>coalesce(?2,0)
             and (a.status in ('prepared','applying','recovery_required') or
               (a.status in ('applied','rolled_back') and exists (
                 select 1 from agent_proposal_application_files f
                 join writing_document_checkpoints p
                   on p.manuscript_id=c.manuscript_id and p.relative_path=f.relative_path
                 left join agent_editor_restorations r
                   on r.application_id=f.application_id and r.relative_path=f.relative_path
                 where f.application_id=a.id and f.target_kind='editor'
                   and ((a.status='applied' and f.phase='acknowledged'
                     and json_extract(p.checkpoint_json,'$.editorReceipt.operationId')=a.id)
                   or (a.status='rolled_back' and f.phase='restored'
                     and json_extract(p.checkpoint_json,'$.editorReceipt.operationId')=r.restore_id))
                   and json_extract(p.checkpoint_json,'$.savedVersion') <
                     coalesce(json_extract(p.checkpoint_json,'$.editorReceipt.stateVersion'),json_extract(p.checkpoint_json,'$.version'))
               )))
             order by a.rowid limit ?3",
).map_err(|_| Flow::original(Error::Unavailable))?;
        let rows = query
            .query_map(
                params![
                    request.manuscript_id.to_string(),
                    request.after,
                    u16::from(request.limit) + 1
                ],
                |r| {
                    Ok((
                        r.get::<_, u64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(|_| Flow::original(Error::Unavailable))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Flow::original(Error::Unavailable))?;
        drop(query);
        let more = rows.len() > usize::from(request.limit);
        let mut items = Vec::new();
        for (id, operation, proposal, conversation) in
            rows.into_iter().take(usize::from(request.limit))
        {
            if id == 0 || id > 9_007_199_254_740_991 {
                return Err(Flow::original(Error::Conflict));
            }
            let parse = |value: &str| {
                Uuid::parse_str(value)
                    .ok()
                    .filter(|id| !id.is_nil() && id.to_string() == value)
                    .ok_or_else(|| Flow::original(Error::Conflict))
            };
            let scope = Scope {
                manuscript_id: request.manuscript_id,
                conversation_id: parse(&conversation)?,
                proposal_id: parse(&proposal)?,
                operation_id: parse(&operation)?,
            };
            // Reuse the full frozen plan / direction validation used by read.
            // This projection contains no bytes, tokens, or dispatch authority.
            let saved = applications::read_in(
                &tx,
                scope.conversation_id,
                scope.manuscript_id,
                scope.proposal_id,
                scope.operation_id,
            )
            .map_err(|error| Flow::original(native::error(error.into())))?;
            let proposal = change_sets::read_file_operation_in(
                &tx,
                scope.conversation_id,
                scope.manuscript_id,
                scope.proposal_id,
            )
            .map_err(|error| Flow::original(native::error(error.into())))?;
            pending_save::validate_with(&tx, &scope, &saved, &proposal.changes, |path| {
                read_checkpoint(
                    &tx,
                    crate::agent::repository::proposal_applications::ApplicationScope {
                        manuscript: scope.manuscript_id,
                        conversation: scope.conversation_id,
                        proposal: scope.proposal_id,
                        operation: saved.operation_id,
                    },
                    path,
                )
            })?;
            items.push(Entry {
                id,
                application: native::project(scope, saved),
            });
        }
        let next_cursor = if more {
            items.last().map(|item| item.id)
        } else {
            None
        };
        Ok(Page {
            schema_version: 1,
            manuscript_id: request.manuscript_id,
            items,
            next_cursor,
        })
    })();
    match result {
        Ok(page) => {
            tx.commit()
                .map_err(|_| Flow::original(Error::Unavailable))?;
            Ok(page)
        }
        Err(Flow::Need(input)) => {
            if tx.rollback().is_err() || !db.is_autocommit() {
                db.require_recovery();
                return Err(Flow::original(Error::RecoveryRequired));
            }
            Err(Flow::Need(input))
        }
        Err(cause) => Err(cause),
    }
}

#[tauri::command]
pub(crate) async fn agent_list_pending_writing_applications(
    store: tauri::State<'_, crate::storage::AppStore>,
    request: ListRequest,
) -> Result<Page, Error> {
    #[cfg(target_os = "macos")]
    {
        list(&store, request).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, request);
        Err(Error::UnsupportedPlatform)
    }
}
