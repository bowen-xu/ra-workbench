//! Archive validates a fixed complete history; SQL admission owns its receipt.
use super::mutation::{mutation_error, proof, status, worker_failure};
use super::*;
use crate::data_interface::SqlMutationError;
use research_agent_core::conversation::CatalogMutationError;
use rusqlite::Transaction;

/// Only this module can construct the proof that the original history was
/// completely validated. The facade still performs the transactional W CAS.
pub(crate) struct ValidatedArchive {
    command: ArchiveConversation,
}
impl ValidatedArchive {
    pub(crate) fn command(&self) -> &ArchiveConversation {
        &self.command
    }
}

impl SqliteConversationCatalog {
    pub(crate) async fn prepare_child_submission(
        &self,
        intent: crate::data_interface::ChildSubmissionIntent,
    ) -> Result<research_agent_core::submission::SubmissionRecord, CatalogMutationError> {
        let operation = intent.operation();
        let connection = self.connection.clone();
        let prepared: crate::data_interface::ChildSubmissionPrepared = self
            .worker_retention
            .spawn(move || connection.prepare_child_submission(&intent))
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    true,
                    "child-submission-worker",
                    "child_submission_worker_result_unknown",
                )
            })?
            .map_err(mutation_error)?;
        if prepared.replayed {
            return Err(worker_failure(
                operation,
                true,
                "child-submission-replay",
                "child_submission_requires_reconciliation",
            ));
        }
        Ok(prepared.record)
    }

    pub(crate) async fn inspect_child_submission(
        &self,
        parent: ConversationId,
        child: ConversationId,
        request: Uuid,
        binding: ConversationBinding,
    ) -> Result<crate::data_interface::ChildSubmissionInspection, CatalogMutationError> {
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || {
                connection.inspect_child_submission(
                    *parent.as_uuid(),
                    *child.as_uuid(),
                    request,
                    &binding,
                )
            })
            .await
            .map_err(|_| {
                worker_failure(
                    request,
                    false,
                    "child-submission-status-worker",
                    "submission_status_worker_failed",
                )
            })?
            .map_err(mutation_error)
    }

    pub(crate) async fn prepare_child_cleanup(
        &self,
        intent: crate::data_interface::ChildCleanupIntent,
        require_absent: bool,
    ) -> Result<(), CatalogMutationError> {
        let operation = crate::data_interface::ChildCleanupIntent::preparation_id(
            *intent.archive.id.as_uuid(),
            intent.request_id,
        );
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || {
                if require_absent {
                    connection.prepare_unsubmitted_child_cleanup(&intent)
                } else {
                    connection.prepare_child_cleanup(&intent)
                }
            })
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    true,
                    "child-cleanup-prepare-worker",
                    "cleanup_prepare_worker_result_unknown",
                )
            })?
            .map_err(mutation_error)?;
        Ok(())
    }

    pub(crate) async fn inspect_child_cleanup(
        &self,
        child: ConversationId,
    ) -> Result<Option<crate::data_interface::ChildCleanupPrepared>, CatalogMutationError> {
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || connection.inspect_child_cleanup(*child.as_uuid()))
            .await
            .map_err(|_| {
                worker_failure(
                    *child.as_uuid(),
                    false,
                    "child-cleanup-status-worker",
                    "cleanup_status_worker_failed",
                )
            })?
            .map_err(mutation_error)
    }

    pub(super) async fn archive_owned(
        &self,
        command: ArchiveConversation,
    ) -> Result<ArchiveProof, CatalogMutationError> {
        command.validate()?;
        let operation = command.operation_id;
        let id = command.id;
        let connection = self.connection.clone();
        let original = command.clone();
        // Unlike inspection, this checks active write admission and drains the
        // old prefix. An exact receipt precedes fresh export, clocks and CAS.
        let replay = self
            .worker_retention
            .spawn(move || connection.archive_replay(&original))
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    false,
                    "archive-replay-worker",
                    "archive_replay_worker_failed",
                )
            })?
            .map_err(mutation_error)?;
        if let Some(receipt) = replay {
            return Ok(proof(id, receipt));
        }
        let prepared = self.validate_archive(command).await?;
        let connection = self.connection.clone();
        let receipt = self
            .worker_retention
            .spawn(move || connection.archive_conversation(&prepared))
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    true,
                    "archive-worker",
                    "archive_worker_result_unknown",
                )
            })?
            .map_err(mutation_error)?;
        Ok(proof(id, receipt))
    }

    pub(crate) async fn validate_archive(
        &self,
        command: ArchiveConversation,
    ) -> Result<ValidatedArchive, CatalogMutationError> {
        command.validate()?;
        if command.archived {
            let snapshot = self
                .export_snapshot(command.id)
                .await
                .map_err(|error| match error {
                    SnapshotExportError::NotFound => CatalogError::NotFound,
                    SnapshotExportError::TooLarge => CatalogError::TooLarge,
                    SnapshotExportError::Corrupt => CatalogError::Corrupt,
                    SnapshotExportError::Unavailable => CatalogError::Unavailable,
                })?;
            if Some(snapshot.watermark) != command.expected_watermark {
                return Err(CatalogError::Conflict.into());
            }
            if snapshot.turns.iter().any(|turn| turn.outcome.is_none()) {
                return Err(CatalogError::Busy.into());
            }
        }
        Ok(ValidatedArchive { command })
    }

    pub(super) async fn inspect_archive(
        &self,
        command: ArchiveConversation,
    ) -> Result<ArchiveStatus, CatalogMutationError> {
        command.validate()?;
        let operation = command.operation_id;
        let id = command.id;
        let connection = self.connection.clone();
        let value = self
            .worker_retention
            .spawn(move || connection.archive_status(&command))
            .await
            .map_err(|_| {
                worker_failure(
                    operation,
                    false,
                    "archive-status-worker",
                    "archive_status_worker_failed",
                )
            })?
            .map_err(mutation_error)?;
        Ok(status(id, value))
    }

    /// Only explicitly unmanaged, old-schema fixtures retain their old API.
    #[cfg(test)]
    pub(crate) async fn archive_legacy_for_tests(
        &self,
        id: ConversationId,
        archived: bool,
    ) -> Result<ConversationSummary, CatalogError> {
        if self.connection.logged() || id.as_uuid().is_nil() {
            return Err(CatalogError::InvalidInput);
        }
        let expected = if archived {
            let snapshot = self
                .export_snapshot(id)
                .await
                .map_err(|error| match error {
                    SnapshotExportError::NotFound => CatalogError::NotFound,
                    SnapshotExportError::TooLarge => CatalogError::TooLarge,
                    SnapshotExportError::Corrupt => CatalogError::Corrupt,
                    SnapshotExportError::Unavailable => CatalogError::Unavailable,
                })?;
            if snapshot.turns.iter().any(|turn| turn.outcome.is_none()) {
                return Err(CatalogError::Busy);
            }
            Some(snapshot.watermark)
        } else {
            None
        };
        self.run(move |db| change_in(db, id, archived, expected))
            .await
    }
}

#[cfg(test)]
fn change_in(
    db: &mut Connection,
    id: ConversationId,
    archived: bool,
    expected: Option<u64>,
) -> Result<ConversationSummary, CatalogError> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let command = ArchiveConversation {
        operation_id: Uuid::new_v4(),
        id,
        archived,
        expected_watermark: expected,
    };
    let updated_at: String = tx
        .query_row("select strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })
        .map_err(db_error)?;
    archive_in(&tx, &command, &updated_at).map_err(|error| match error {
        SqlMutationError::Domain(error) => error,
        SqlMutationError::Sql(_) => CatalogError::Unavailable,
    })?;
    let summary=decode(tx.query_row("select id,title,archived,durable_count,created_at,updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=agent_conversations.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=agent_conversations.id)) from agent_conversations where id=?1", [id.as_uuid().to_string()], read_row).map_err(db_error)?)?;
    tx.commit().map_err(db_error)?;
    Ok(summary)
}

pub(crate) fn check_archive_in(
    tx: &Transaction<'_>,
    command: &ArchiveConversation,
) -> Result<(), SqlMutationError> {
    command.validate()?;
    let durable: i64 = tx
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [command.id.as_uuid().to_string()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(CatalogError::NotFound)?;
    let durable = u64::try_from(durable).map_err(|_| CatalogError::Corrupt)?;
    if durable > 9_007_199_254_740_991 {
        return Err(CatalogError::Corrupt.into());
    }
    if command.archived && command.expected_watermark != Some(durable) {
        return Err(CatalogError::Conflict.into());
    }
    Ok(())
}

pub(crate) fn archive_in(
    tx: &Transaction<'_>,
    command: &ArchiveConversation,
    updated_at: &str,
) -> Result<(), SqlMutationError> {
    check_archive_in(tx, command)?;
    tx.execute(
        "update agent_conversations set archived=?2,updated_at=?3 where id=?1 and archived<>?2",
        params![
            command.id.as_uuid().to_string(),
            command.archived,
            updated_at
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "archive/tests.rs"]
mod tests;
