//! A compact preparation's history fence belongs to its original submission.
use super::*;
use rusqlite::OptionalExtension;

pub(crate) struct PreparedWritingCompaction {
    pub submission: PreparedWritingSubmission,
    pub expected_revision: u64,
}

impl WritingSubmissionService {
    /// Recover the original receipt/header before preparing the exact saved
    /// model. No current editor/file request or new history fence is accepted.
    pub(crate) async fn restore_compaction(
        &self,
        models: &AgentModelResolver,
        receipt: WritingSubmissionReceipt,
    ) -> Result<PreparedWritingCompaction, WritingSubmissionError> {
        let store = self.store.clone();
        let expected_revision = self
            .worker_retention
            .spawn(move || {
                let db = store
                    .connection()
                    .map_err(|_| WritingSubmissionError::Unavailable)?;
                check_binding(&db, receipt.conversation_id, receipt.manuscript_id)?;
                let record = read(&db, receipt.conversation_id, receipt.request_id)?
                    .ok_or(SubmissionError::NotFound)?;
                validate_scope(&record, receipt.manuscript_id)?;
                if record.input.text.trim() != "/compact" {
                    return Err(SubmissionError::Conflict.into());
                }
                let revision = read_in(&db, receipt.conversation_id, receipt.request_id)?
                    .ok_or(SubmissionError::NotFound)?;
                Ok::<_, WritingSubmissionError>(revision)
            })
            .await
            .map_err(|_| WritingSubmissionError::Unavailable)??;
        let submission = self.restore(models, receipt).await?;
        Ok(PreparedWritingCompaction {
            submission,
            expected_revision,
        })
    }
}

/// Legacy native hosts retain their explicit-fence contract without fabricating
/// metadata for old records. Missing headers are not restorable preparations.
pub(crate) fn read_in(
    db: &Connection,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<u64>, WritingSubmissionError> {
    let exists: bool = db.query_row(
        "select exists(select 1 from sqlite_schema where type='table' and name='agent_compaction_preparations')",
        [], |row| row.get(0),
    ).map_err(WritingSubmissionError::Sql)?;
    if !exists {
        let version: u64 = db
            .query_row(
                "select coalesce(max(version),0) from schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(WritingSubmissionError::Sql)?;
        return if version < 213 {
            Ok(None)
        } else {
            Err(SubmissionError::Corrupt.into())
        };
    }
    let revision: Option<u64> = db.query_row(
        "select expected_revision from agent_compaction_preparations where conversation_id=?1 and request_id=?2",
        params![conversation.to_string(), request.to_string()], |row| row.get(0),
    ).optional().map_err(WritingSubmissionError::Sql)?;
    if revision.is_some_and(|value| value > 9_007_199_254_740_991) {
        return Err(SubmissionError::Corrupt.into());
    }
    Ok(revision)
}

pub(crate) fn check_prepare_in(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    text: &str,
    expected: Option<u64>,
) -> Result<(), WritingSubmissionError> {
    let saved = read_in(tx, conversation, request)?;
    match (expected, saved) {
        (None, Some(_)) => return Err(SubmissionError::Conflict.into()),
        (Some(expected), Some(saved)) if expected != saved => {
            return Err(SubmissionError::Conflict.into());
        }
        (Some(expected), None) => {
            let existing =
                crate::agent::repository::submissions::read_sql(tx, conversation, request)
                    .map_err(|cause| match cause {
                        crate::agent::repository::SqlRepositoryError::Domain(cause) => {
                            WritingSubmissionError::Input(cause)
                        }
                        crate::agent::repository::SqlRepositoryError::Sql(cause) => {
                            WritingSubmissionError::Sql(cause)
                        }
                    })?;
            if text.trim() != "/compact" || existing.is_some() {
                return Err(SubmissionError::Conflict.into());
            }
            let actual: u64 = tx
                .query_row(
                    "select durable_count from agent_conversations where id=?1",
                    [conversation.to_string()],
                    |row| row.get(0),
                )
                .map_err(WritingSubmissionError::Sql)?;
            if actual != expected {
                return Err(SubmissionError::Conflict.into());
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn freeze_in(
    tx: &rusqlite::Transaction<'_>,
    record: &SubmissionRecord,
    expected: Option<u64>,
) -> Result<(), WritingSubmissionError> {
    if let Some(expected) = expected {
        if let Some(saved) = read_in(tx, record.conversation_id, record.request_id)? {
            if saved != expected {
                return Err(SubmissionError::Conflict.into());
            }
        } else {
            tx.execute(
                "insert into agent_compaction_preparations(conversation_id,request_id,expected_revision) values(?1,?2,?3)",
                params![record.conversation_id.to_string(),record.request_id.to_string(),expected],
            ).map_err(WritingSubmissionError::Sql)?;
        }
    }
    Ok(())
}
