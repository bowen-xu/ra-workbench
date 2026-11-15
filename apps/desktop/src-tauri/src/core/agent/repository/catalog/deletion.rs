//! Conversation deletion shares SQL admission; workspace cleanup stays external.
use super::mutation::{mutation_error, status, worker_failure};
use super::*;
use crate::data_interface::DeletionFailure;
use research_agent_core::conversation::{
    CatalogMutationError, CatalogMutationFuture, CatalogMutationStatus,
    ConversationDeletionError as Error, ConversationDeletionFuture,
    ConversationDeletionReceipt as Receipt, ConversationDeletionRequest as Request,
    ConversationDeletionStore, ConversationRetention as Retention,
};
use sha2::{Digest, Sha256};
#[path = "deletion/publication.rs"]
mod publication;
pub(crate) use publication::delete_in;
#[path = "deletion/workspace.rs"]
mod workspace;
use workspace::{WorkspaceRow, purge_workspace_files, workspaces};
#[cfg(test)]
#[path = "deletion/legacy.rs"]
mod legacy;
#[cfg(test)]
#[path = "deletion/tests.rs"]
mod tests;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DeletionSqlError {
    #[error(transparent)]
    Domain(#[from] Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}
impl From<CatalogError> for DeletionSqlError {
    fn from(error: CatalogError) -> Self {
        Self::Domain(error.into())
    }
}
pub(crate) struct PreparedDeletion {
    request: Request,
    hash: String,
    workspaces: Vec<WorkspaceRow>,
    cleanup_complete: bool,
    #[cfg(unix)]
    _workspace_guards: Vec<crate::agent::workspace::WorkspaceLease>,
}
impl PreparedDeletion {
    pub(crate) fn request(&self) -> &Request {
        &self.request
    }
    pub(crate) fn cleanup_complete(&self) -> bool {
        self.cleanup_complete
    }
}
pub(crate) fn deletion_request_hash(request: &Request) -> String {
    // Preserve the exact deployed tombstone hash; managed receipts add their own family/version.
    let value = serde_json::json!({"version":1,"requestId":request.request_id,"conversationId":request.conversation_id.as_uuid(),
        "bindingKind":request.binding.as_ref().map(|b|binding_kind(b.kind)),"resourceId":request.binding.as_ref().map(|b|&b.resource_id),
        "title":request.expected_title,"watermark":request.expected_durable_count,"force":request.force});
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
use deletion_request_hash as request_hash;
pub(crate) fn deletion_replay_sql(
    db: &Connection,
    request: &Request,
) -> Result<Option<Receipt>, DeletionSqlError> {
    replay_sql(db, request, &request_hash(request))
}
fn replay_sql(
    db: &Connection,
    request: &Request,
    hash: &str,
) -> Result<Option<Receipt>, DeletionSqlError> {
    let saved: Option<(String,String)> = db.query_row("select conversation_id,request_hash from agent_conversation_deletions where request_id=?1",
        [request.request_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((id, saved_hash)) = saved else {
        return Ok(None);
    };
    if id != request.conversation_id.as_uuid().to_string() || saved_hash != hash {
        return Err(CatalogError::Conflict.into());
    }
    Ok(Some(receipt(request)))
}
fn receipt(request: &Request) -> Receipt {
    Receipt {
        request_id: request.request_id,
        conversation_id: request.conversation_id,
    }
}
fn check_summary(summary: &ConversationSummary, request: &Request) -> Result<(), Error> {
    if !summary.archived {
        return Err(Error::NotArchived);
    }
    if summary.title != request.expected_title
        || summary.durable_count != request.expected_durable_count
    {
        return Err(CatalogError::Conflict.into());
    }
    Ok(())
}
fn mutation_failure(cause: CatalogMutationError) -> Error {
    match cause {
        CatalogMutationError::Domain(cause) => cause.into(),
        CatalogMutationError::Data(cause) => cause.into(),
    }
}
fn workspace_failure(
    operation: Uuid,
    kind: research_agent_core::conversation::CatalogDataFailureKind,
    phase: &'static str,
    cause: &'static str,
) -> Error {
    use research_agent_core::conversation::{CatalogDataFailure, CatalogMutationDiagnostic};
    CatalogDataFailure {
        kind,
        diagnostic: Box::new(CatalogMutationDiagnostic {
            operation_id: operation,
            epoch: None,
            sql_fence: None,
            phase,
            sqlite_code: None,
            sqlite_extended_code: None,
            safe_cause: cause,
        }),
    }
    .into()
}
fn failure(cause: DeletionFailure) -> Error {
    match cause {
        DeletionFailure::Domain(cause) => cause,
        DeletionFailure::Storage(cause) => mutation_failure(mutation_error(cause)),
        DeletionFailure::CleanupComplete(cause) => {
            use research_agent_core::conversation::{
                CatalogDataFailure, CatalogDataFailureKind, CatalogMutationDiagnostic,
            };
            let d = *cause;
            CatalogDataFailure {
                kind: CatalogDataFailureKind::Partial,
                diagnostic: Box::new(CatalogMutationDiagnostic {
                    operation_id: d.operation_id,
                    epoch: d.epoch,
                    sql_fence: d.sql_fence,
                    phase: d.phase,
                    sqlite_code: d.sqlite_code,
                    sqlite_extended_code: d.sqlite_extended_code,
                    safe_cause: d.safe_cause,
                }),
            }
            .into()
        }
    }
}
fn preparation_failure(operation: Uuid, cause: DeletionSqlError) -> Error {
    match cause {
        DeletionSqlError::Domain(cause) => cause,
        DeletionSqlError::Sql(cause) => {
            use research_agent_core::conversation::{
                CatalogDataFailure, CatalogDataFailureKind, CatalogMutationDiagnostic,
            };
            let (code, extended) = match cause {
                rusqlite::Error::SqliteFailure(code, _) => {
                    (Some(format!("{:?}", code.code)), Some(code.extended_code))
                }
                _ => (None, None),
            };
            CatalogDataFailure {
                kind: CatalogDataFailureKind::Unavailable,
                diagnostic: Box::new(CatalogMutationDiagnostic {
                    operation_id: operation,
                    epoch: None,
                    sql_fence: None,
                    phase: "delete-workspace-read",
                    sqlite_code: code,
                    sqlite_extended_code: extended,
                    safe_cause: "delete_workspace_read_unavailable",
                }),
            }
            .into()
        }
    }
}
#[cfg(test)]
fn domain_or(cause: DeletionSqlError) -> Error {
    match cause {
        DeletionSqlError::Domain(cause) => cause,
        DeletionSqlError::Sql(cause) => db_error(cause).into(),
    }
}
impl ConversationDeletionStore for SqliteConversationCatalog {
    fn delete_in_epoch(
        &self,
        request: Request,
        expected_epoch: Uuid,
    ) -> ConversationDeletionFuture<'_> {
        Box::pin(async move {
            request.validate()?;
            self.check_deletion_epoch(request.request_id, expected_epoch)
                .await?;
            // All later workers retain this same immutable SQL/log pair and root.
            self.delete(request).await
        })
    }
    fn delete(&self, request: Request) -> ConversationDeletionFuture<'_> {
        Box::pin(async move {
            request.validate()?;
            if self.replay_deletion(request.clone()).await? {
                return Ok(receipt(&request));
            }
            let prepared = match self.prepare_deletion(request.clone()).await {
                Ok(prepared) => prepared,
                Err(cause) => {
                    // Another identical publication may have won during export or cleanup.
                    if self.replay_deletion(request.clone()).await? {
                        return Ok(receipt(&request));
                    }
                    return Err(cause);
                }
            };
            let connection = self.connection.clone();
            let operation = request.request_id;
            self.worker_retention
                .spawn(move || connection.delete_conversation(&prepared))
                .await
                .map_err(|_| {
                    mutation_failure(worker_failure(
                        operation,
                        true,
                        "delete-worker",
                        "delete_worker_result_unknown",
                    ))
                })?
                .map_err(failure)?;
            Ok(receipt(&request))
        })
    }
    fn deletion_status(
        &self,
        request: Request,
    ) -> CatalogMutationFuture<'_, CatalogMutationStatus> {
        Box::pin(async move {
            request.validate()?;
            let operation = request.request_id;
            let id = request.conversation_id;
            let connection = self.connection.clone();
            let result = self
                .worker_retention
                .spawn(move || connection.deletion_status(&request))
                .await
                .map_err(|_| {
                    worker_failure(
                        operation,
                        false,
                        "delete-status-worker",
                        "delete_status_worker_failed",
                    )
                })?
                .map_err(mutation_error)?;
            Ok(status(id, result))
        })
    }
}
impl SqliteConversationCatalog {
    /// Reclaim only copies whose conversation deletion has already committed.
    /// The durable queue survives interruption without replaying the deletion.
    pub(crate) fn collect_deleted_workspaces(&self) -> Result<(), Error> {
        use research_agent_core::workspace::WorkspaceIdentity;
        let db = self
            .connection
            .lock()
            .map_err(|_| CatalogError::Unavailable)?;
        let mut statement = db.prepare("select g.operation_id,g.conversation_id,g.turn_id,g.manuscript_id,g.lease_id from agent_deleted_workspace_gc g where exists(select 1 from agent_conversation_deletions d where d.request_id=g.operation_id and d.conversation_id=g.conversation_id) and not exists(select 1 from agent_conversations c where c.id=g.conversation_id) order by g.operation_id,g.lease_id").map_err(|_| CatalogError::Unavailable)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| CatalogError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| CatalogError::Unavailable)?;
        drop(statement);
        drop(db);
        if rows.is_empty() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            use crate::agent::workspace::WorkspaceStore;
            let root = self
                .workspace_directory
                .as_ref()
                .ok_or(CatalogError::Unavailable)?
                .canonicalize()
                .map_err(|_| CatalogError::Unavailable)?;
            let store =
                WorkspaceStore::open(&root, |_| Ok(())).map_err(|_| CatalogError::Unavailable)?;
            for (operation, conversation, turn, manuscript, lease) in rows {
                let parse = |value: &str| Uuid::parse_str(value).map_err(|_| CatalogError::Corrupt);
                let identity = WorkspaceIdentity {
                    conversation_id: parse(&conversation)?,
                    turn_id: parse(&turn)?,
                    environment_id: format!("manuscript:{manuscript}"),
                    lease_id: parse(&lease)?,
                };
                if store.discard(&identity).is_err() {
                    continue;
                }
                self.connection
                    .complete_deleted_workspace_gc(
                        parse(&operation)?,
                        parse(&conversation)?,
                        parse(&lease)?,
                    )
                    .map_err(|cause| mutation_failure(mutation_error(cause)))?;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = rows;
            return Err(CatalogError::Unavailable.into());
        }
        Ok(())
    }
    pub(crate) fn pending_deleted_workspace_operations(
        &self,
    ) -> Result<std::collections::HashSet<Uuid>, Error> {
        let db = self
            .connection
            .lock()
            .map_err(|_| CatalogError::Unavailable)?;
        let mut statement = db
            .prepare("select distinct operation_id from agent_deleted_workspace_gc")
            .map_err(|_| CatalogError::Unavailable)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| CatalogError::Unavailable)?;
        let mut pending = std::collections::HashSet::new();
        for row in rows {
            pending.insert(
                Uuid::parse_str(&row.map_err(|_| CatalogError::Unavailable)?)
                    .map_err(|_| CatalogError::Corrupt)?,
            );
        }
        Ok(pending)
    }
    pub(crate) async fn publish_deletion_batch(
        &self,
        prepared: Vec<PreparedDeletion>,
    ) -> Result<(), Error> {
        let operation = prepared
            .first()
            .ok_or(CatalogError::InvalidInput)?
            .request
            .request_id;
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || connection.delete_conversations_batch(&prepared))
            .await
            .map_err(|_| {
                mutation_failure(worker_failure(
                    operation,
                    true,
                    "delete-batch-worker",
                    "delete_batch_result_unknown",
                ))
            })?
            .map_err(failure)
    }
    pub(crate) async fn check_deletion_epoch(
        &self,
        operation: Uuid,
        expected: Uuid,
    ) -> Result<(), Error> {
        if operation.is_nil() || expected.is_nil() {
            return Err(CatalogError::InvalidInput.into());
        }
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || connection.check_deletion_epoch(operation, expected))
            .await
            .map_err(|_| {
                mutation_failure(worker_failure(
                    operation,
                    false,
                    "delete-epoch-worker",
                    "delete_epoch_worker_failed",
                ))
            })?
            .map_err(|cause| mutation_failure(mutation_error(cause)))
    }
    async fn replay_deletion(&self, request: Request) -> Result<bool, Error> {
        let operation = request.request_id;
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || connection.deletion_replay(&request))
            .await
            .map_err(|_| {
                mutation_failure(worker_failure(
                    operation,
                    false,
                    "delete-replay-worker",
                    "delete_replay_worker_failed",
                ))
            })?
            .map_err(failure)
    }
    pub(crate) async fn prepare_deletion(
        &self,
        request: Request,
    ) -> Result<PreparedDeletion, Error> {
        self.prepare_deletion_with_cleanup(request, true).await
    }
    pub(crate) async fn prepare_batch_deletion(
        &self,
        request: Request,
    ) -> Result<PreparedDeletion, Error> {
        self.prepare_deletion_with_cleanup(request, false).await
    }
    async fn prepare_deletion_with_cleanup(
        &self,
        request: Request,
        cleanup: bool,
    ) -> Result<PreparedDeletion, Error> {
        request.validate()?;
        let summary = self
            .read(request.conversation_id, request.binding.clone())
            .await?;
        check_summary(&summary, &request)?;
        let snapshot = self
            .export_snapshot(request.conversation_id)
            .await
            .map_err(|error| match error {
                SnapshotExportError::NotFound => CatalogError::NotFound,
                SnapshotExportError::TooLarge => CatalogError::TooLarge,
                SnapshotExportError::Corrupt => CatalogError::Corrupt,
                SnapshotExportError::Unavailable => CatalogError::Unavailable,
            })?;
        if snapshot.turns.iter().any(|turn| turn.outcome.is_none()) {
            return Err(CatalogError::Busy.into());
        }
        if snapshot.watermark != request.expected_durable_count {
            return Err(CatalogError::Conflict.into());
        }
        let root = self.workspace_directory.clone();
        let connection = self.connection.clone();
        let operation = request.request_id;
        self.worker_retention
            .spawn(move || {
                // Keep existing SQL/admission ordering through external cleanup, but do
                // not execute files from the replayable SQL transaction body.
                let mut db = connection.lock().map_err(|_| {
                    mutation_failure(worker_failure(
                        operation,
                        false,
                        "delete-workspace-read",
                        "delete_workspace_read_unavailable",
                    ))
                })?;
                if !crate::data_interface::Database::deletion_cleanup_needed(&mut db, &request)
                    .map_err(failure)?
                {
                    return Ok(PreparedDeletion {
                        hash: request_hash(&request),
                        request,
                        workspaces: vec![],
                        cleanup_complete: false,
                        #[cfg(unix)]
                        _workspace_guards: Vec::new(),
                    });
                }
                publication::check_state_sql(&db, &request)
                    .map_err(|cause| preparation_failure(operation, cause))?;
                let rows = workspaces(&db, request.conversation_id)
                    .map_err(|cause| preparation_failure(operation, cause))?;
                #[cfg(unix)]
                let guards = if request.force && !cleanup {
                    workspace::hold_workspace_files(&rows, &request, root.clone())?
                } else {
                    Vec::new()
                };
                #[cfg(not(unix))]
                if request.force && !cleanup && !rows.is_empty() {
                    return Err(CatalogError::Unavailable.into());
                }
                if request.force && cleanup {
                    purge_workspace_files(&rows, &request, root)?;
                }
                Ok(PreparedDeletion {
                    hash: request_hash(&request),
                    cleanup_complete: request.force && cleanup && !rows.is_empty(),
                    #[cfg(unix)]
                    _workspace_guards: guards,
                    request,
                    workspaces: rows,
                })
            })
            .await
            .map_err(|_| {
                workspace_failure(
                    operation,
                    research_agent_core::conversation::CatalogDataFailureKind::ExternalUnknown,
                    "delete-workspace-worker",
                    "delete_workspace_result_unknown",
                )
            })?
    }
}
