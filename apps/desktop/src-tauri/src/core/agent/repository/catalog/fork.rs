//! Fork publication uses the same admission/receipt owner as the journal.
use super::mutation::{mutation_error, status, worker_failure};
use super::*;
use research_agent_core::conversation::{
    CatalogMutationError, CatalogMutationFuture, CatalogMutationStatus, ConversationFork,
    ConversationForkRequest, ConversationForkStore, materialize_fork_history,
};
#[path = "fork/publication.rs"]
mod publication;
pub(crate) use publication::{PreparedFork, fork_in};
#[cfg(test)]
#[path = "fork/legacy.rs"]
mod legacy;

pub(crate) fn validate_fork_request(request: &ConversationForkRequest) -> Result<(), CatalogError> {
    if request.child_id.as_uuid().is_nil()
        || request.parent_id.as_uuid().is_nil()
        || request.child_id == request.parent_id
        || request.through_turn_id.is_nil()
    {
        return Err(CatalogError::InvalidInput);
    }
    validate_binding(&request.binding)
}
impl ConversationForkStore for SqliteConversationCatalog {
    fn fork_in_epoch(
        &self,
        request: ConversationForkRequest,
        expected_epoch: Uuid,
    ) -> CatalogMutationFuture<'_, ConversationFork> {
        Box::pin(async move {
            validate_fork_request(&request)?;
            if expected_epoch.is_nil() {
                return Err(CatalogError::InvalidInput.into());
            }
            let operation = *request.child_id.as_uuid();
            let connection = self.connection.clone();
            self.worker_retention
                .spawn(move || connection.check_fork_epoch(operation, expected_epoch))
                .await
                .map_err(|_| {
                    worker_failure(
                        operation,
                        false,
                        "fork-epoch-worker",
                        "fork_epoch_worker_failed",
                    )
                })?
                .map_err(mutation_error)?;
            // This catalog's connection cannot be retargeted. The same retained
            // SQL/Raft pair is used in every worker below, including replay.
            self.fork(request).await
        })
    }
    fn fork(
        &self,
        request: ConversationForkRequest,
    ) -> CatalogMutationFuture<'_, ConversationFork> {
        Box::pin(async move {
            validate_fork_request(&request)?;
            let operation = *request.child_id.as_uuid();
            let probe = request.clone();
            let connection = self.connection.clone();
            let original = self
                .worker_retention
                .spawn(move || connection.fork_replay(&probe))
                .await
                .map_err(|_| {
                    worker_failure(
                        operation,
                        false,
                        "fork-replay-worker",
                        "fork_replay_worker_failed",
                    )
                })?
                .map_err(mutation_error)?;
            if original.is_some() {
                return self.fork_result(request).await;
            }
            // Existing pre-facade lineage remains readable, never a new proof.
            let legacy = request.clone();
            if let Some(saved) = self
                .run(move |db| replay(db, &legacy))
                .await
                .map_err(|error| match error {
                    CatalogError::Unavailable => {
                        worker_failure(operation, false, "fork-lookup", "fork_lookup_unavailable")
                    }
                    error => error.into(),
                })?
            {
                return Ok(saved);
            }
            let prepared =
                self.prepare_fork(request.clone())
                    .await
                    .map_err(|error| match error {
                        CatalogError::Unavailable => worker_failure(
                            operation,
                            false,
                            "fork-prepare",
                            "fork_prepare_unavailable",
                        ),
                        error => error.into(),
                    })?;
            let connection = self.connection.clone();
            self.worker_retention
                .spawn(move || connection.fork_conversation(&prepared))
                .await
                .map_err(|_| {
                    worker_failure(operation, true, "fork-worker", "fork_worker_result_unknown")
                })?
                .map_err(mutation_error)?;
            self.fork_result(request).await
        })
    }
    fn fork_status(
        &self,
        request: ConversationForkRequest,
    ) -> CatalogMutationFuture<'_, CatalogMutationStatus> {
        Box::pin(async move {
            validate_fork_request(&request)?;
            let operation = *request.child_id.as_uuid();
            let id = request.child_id;
            let connection = self.connection.clone();
            let result = self
                .worker_retention
                .spawn(move || connection.fork_status(&request))
                .await
                .map_err(|_| {
                    worker_failure(
                        operation,
                        false,
                        "fork-status-worker",
                        "fork_status_worker_failed",
                    )
                })?
                .map_err(mutation_error)?;
            Ok(status(id, result))
        })
    }
}
impl SqliteConversationCatalog {
    async fn fork_result(
        &self,
        request: ConversationForkRequest,
    ) -> Result<ConversationFork, CatalogMutationError> {
        let operation = *request.child_id.as_uuid();
        self.run(move |db| replay(db, &request)?.ok_or(CatalogError::NotFound))
            .await
            .map_err(|error| match error {
                CatalogError::Unavailable => worker_failure(
                    operation,
                    true,
                    "fork-result-read",
                    "fork_result_unavailable",
                ),
                error => error.into(),
            })
    }
    async fn prepare_fork(
        &self,
        request: ConversationForkRequest,
    ) -> Result<PreparedFork, CatalogError> {
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || {
                let (summary, history) = {
                    let mut db = connection.lock().map_err(|_| CatalogError::Unavailable)?;
                    export::read_snapshot(&mut db, request.parent_id).map_err(
                        |error| match error {
                            SnapshotExportError::NotFound => CatalogError::NotFound,
                            SnapshotExportError::TooLarge => CatalogError::TooLarge,
                            SnapshotExportError::Corrupt => CatalogError::Corrupt,
                            SnapshotExportError::Unavailable => CatalogError::Unavailable,
                        },
                    )?
                };
                PreparedFork::prepare(request, summary, &history)
            })
            .await
            .map_err(|_| CatalogError::Unavailable)?
    }
}

fn replay(
    db: &Connection,
    request: &ConversationForkRequest,
) -> Result<Option<ConversationFork>, CatalogError> {
    let child = request.child_id.as_uuid().to_string();
    let row = db.query_row("select id,title,archived,durable_count,created_at,updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=agent_conversations.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=agent_conversations.id)) from agent_conversations where id=?1",
        [&child], read_row).optional().map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let lineage: Option<(String, String, i64, String, String)> = db.query_row(
        "select parent_id,through_turn_id,parent_end_seq,resource_kind,resource_id from agent_conversation_forks where child_id=?1",
        [&child], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
    ).optional().map_err(db_error)?;
    let Some((parent, turn, end, kind, resource)) = lineage else {
        return Err(CatalogError::Conflict);
    };
    if parent != request.parent_id.as_uuid().to_string()
        || turn != request.through_turn_id.to_string()
        || kind != binding_kind(request.binding.kind)
        || resource != request.binding.resource_id
    {
        return Err(CatalogError::Conflict);
    }
    if end <= 0 {
        return Err(CatalogError::Corrupt);
    }
    Ok(Some(ConversationFork {
        conversation: decode(row)?,
        parent_id: request.parent_id,
        through_turn_id: request.through_turn_id,
        parent_end_seq: end as u64,
    }))
}

#[cfg(test)]
#[path = "fork/tests.rs"]
mod tests;
