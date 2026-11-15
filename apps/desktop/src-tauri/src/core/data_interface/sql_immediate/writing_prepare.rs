//! One owned transaction freezes ordinary writing and manual compact preparation.
use super::*;
use crate::agent::writing_context::ContextNeed;
use crate::agent::{repository::submissions, writing_submission};
use crate::{
    agent::writing_context::WritingContextReader, ai::structured_resource_validation::Flow,
};
use research_agent_core::{conversation::CatalogError, submission::SubmissionRecord};
use rusqlite::Connection;
#[path = "writing_prepare/types.rs"]
mod types;
pub(crate) use types::FAMILY as WRITING_PREPARE_FAMILY;
use types::identity;
pub(crate) use types::{WritingPrepareIntent, WritingPrepareSource, WritingPrepareStatus};
#[path = "writing_prepare/mutation.rs"]
mod mutation;
#[path = "writing_prepare/status.rs"]
mod status;

fn failure(
    command: &WritingPrepareIntent,
    fence: ReadFence,
    phase: &'static str,
    reason: &'static str,
) -> SqlImmediateFailure {
    SqlImmediateFailure::Corrupt(Box::new(diagnostic(
        command.operation(),
        fence,
        phase,
        reason,
    )))
}
fn record_in(
    db: &Connection,
    command: &WritingPrepareIntent,
    fence: ReadFence,
) -> Result<Option<SubmissionRecord>, SqlImmediateFailure> {
    use crate::agent::repository::SqlRepositoryError as E;
    submissions::read_sql(db, command.conversation(), command.operation()).map_err(|cause| {
        match cause {
            E::Sql(cause) => {
                receipts::storage(command.operation(), fence, "writing-prepare-read", &cause)
            }
            E::Domain(_) => failure(
                command,
                fence,
                "writing-prepare-read",
                "writing_prepare_record_invalid",
            ),
        }
    })
}
enum PreparedFact {
    Present(Box<SubmissionRecord>),
    Deleted,
}
fn committed_in(
    db: &Connection,
    command: &WritingPrepareIntent,
    fence: ReadFence,
) -> Result<PreparedFact, SqlImmediateFailure> {
    let record = match record_in(db, command, fence)? {
        Some(record) => record,
        None => {
            // Only an exact immutable proof plus the original deletion fact may
            // explain a missing result. A tombstone alone proves no preparation.
            let deleted:bool=db.query_row("select exists(select 1 from agent_conversation_deletions where conversation_id=?1) and not exists(select 1 from agent_conversations where id=?1)",
                [command.conversation().to_string()],|row|row.get(0))
                .map_err(|cause|receipts::storage(command.operation(),fence,"writing-prepare-replay",&cause))?;
            if deleted {
                return Ok(PreparedFact::Deleted);
            }
            return Err(failure(
                command,
                fence,
                "writing-prepare-replay",
                "writing_prepare_record_missing",
            ));
        }
    };
    writing_submission::validate_scope(&record, command.manuscript()).map_err(|_| {
        failure(
            command,
            fence,
            "writing-prepare-replay",
            "writing_prepare_scope_invalid",
        )
    })?;
    let compact = writing_submission::read_compaction_revision_in(
        db,
        command.conversation(),
        command.operation(),
    )
    .map_err(|cause| match cause {
        writing_submission::WritingSubmissionError::Sql(cause) => {
            receipts::storage(command.operation(), fence, "writing-prepare-replay", &cause)
        }
        _ => failure(
            command,
            fence,
            "writing-prepare-replay",
            "writing_prepare_purpose_invalid",
        ),
    })?;
    let grants: bool = db
        .query_row(
            "select exists(select 1 from agent_turn_resource_grants where turn_id=?1)",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|cause| {
            receipts::storage(command.operation(), fence, "writing-prepare-replay", &cause)
        })?;
    if compact != command.compact_revision || record.input.text != command.request.text || !grants {
        return Err(failure(
            command,
            fence,
            "writing-prepare-replay",
            "writing_prepare_result_invalid",
        ));
    }
    Ok(PreparedFact::Present(Box::new(record)))
}
fn authorize(
    db: &Connection,
    command: &WritingPrepareIntent,
    fence: ReadFence,
) -> Result<(), SqlImmediateFailure> {
    writing_submission::check_binding(db, command.conversation(), command.manuscript())
        .map_err(|cause| mutation::materialization_error(command, fence, cause))
}
fn replay_in(
    db: &Connection,
    command: &WritingPrepareIntent,
    fence: ReadFence,
    receipt: Option<SqlImmediateReceipt>,
) -> Result<Option<SubmissionRecord>, SqlImmediateFailure> {
    if receipt.is_some() {
        return match committed_in(db, command, fence)? {
            PreparedFact::Present(record) => Ok(Some(*record)),
            PreparedFact::Deleted => Err(SqlImmediateFailure::Domain(CatalogError::NotFound)),
        };
    }
    if record_in(db, command, fence)?.is_some() {
        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
            command.operation(),
            fence,
            "writing-prepare-replay",
            "legacy_prepare_without_intent_proof",
        ))));
    }
    authorize(db, command, fence)?;
    Ok(None)
}
impl Database {
    /// Only fresh preparation crosses the credential/source boundary. Existing
    /// immutable proof is not permission to start or to use a live resource.
    pub(crate) fn replay_writing_prepare(
        &self,
        command: &WritingPrepareIntent,
    ) -> Result<Option<SubmissionRecord>, SqlImmediateFailure> {
        let identity = identity(command)?;
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| unmanaged(command.operation()))?;
        let mut guard = owner
            .materialize_guard()
            .map_err(|cause| admission_error(command.operation(), cause, None))?;
        let fence = guard.fence();
        let db = self.raw.lock().map_err(|_| {
            guard.block();
            failure(
                command,
                fence,
                "writing-prepare-replay",
                "sql_mutex_poisoned",
            )
        })?;
        if !db.is_autocommit() {
            guard.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                command.operation(),
                fence,
                "writing-prepare-replay",
                "sql_transaction_already_open",
            ))));
        }
        let result = (|| {
            receipts::check_state(&db, command.operation(), fence, "writing-prepare-replay")?;
            receipts::check_cross_kind(&db, command.operation(), fence, "writing-prepare-replay")?;
            let receipt = receipts::lookup(&db, &identity, fence, "writing-prepare-replay")?;
            replay_in(&db, command, fence, receipt)
        })();
        if matches!(&result, Err(SqlImmediateFailure::Corrupt(_))) {
            guard.block();
        }
        result
    }
    pub(crate) fn prepare_writing(
        &self,
        command: &WritingPrepareIntent,
        source: WritingPrepareSource,
    ) -> Result<SubmissionRecord, SqlImmediateFailure> {
        self.prepare_writing_owned(
            command,
            source,
            #[cfg(test)]
            Fault::None,
        )
    }
    fn prepare_writing_owned(
        &self,
        command: &WritingPrepareIntent,
        source: WritingPrepareSource,
        #[cfg(test)] fault: Fault,
    ) -> Result<SubmissionRecord, SqlImmediateFailure> {
        self.prepare_writing_attempt(
            command,
            source,
            None,
            #[cfg(test)]
            fault,
        )
        .map_err(|flow| match flow {
            Flow::Fail(failure) => failure.error,
            Flow::Need(_) => unreachable!("static writing preparation cannot need Native"),
        })
    }
    pub(crate) fn prepare_writing_context(
        &self,
        command: &WritingPrepareIntent,
        source: WritingPrepareSource,
        writing: &mut dyn WritingContextReader,
    ) -> Result<SubmissionRecord, Flow<SqlImmediateFailure, ContextNeed>> {
        self.prepare_writing_attempt(
            command,
            source,
            Some(writing),
            #[cfg(test)]
            Fault::None,
        )
    }
    fn prepare_writing_attempt(
        &self,
        command: &WritingPrepareIntent,
        source: WritingPrepareSource,
        writing: Option<&mut dyn WritingContextReader>,
        #[cfg(test)] fault: Fault,
    ) -> Result<SubmissionRecord, Flow<SqlImmediateFailure, ContextNeed>> {
        let identity = identity(command).map_err(Flow::original)?;
        let mut result = None;
        let mut pause =
            writing_source::SourcePause::new(command.operation(), "writing-prepare-source-pause");
        let executed = self.execute_sql(
            command.operation(),
            |tx, fence| {
                receipts::check_state(tx, command.operation(), fence, "writing-prepare")?;
                receipts::check_cross_kind(tx, command.operation(), fence, "writing-prepare")?;
                let receipt = receipts::lookup(tx, &identity, fence, "writing-prepare")?;
                if let Some(record) = replay_in(tx, command, fence, receipt.clone())? {
                    result = Some(record);
                    return Ok(receipt.expect("replay requires an exact receipt"));
                }
                result = Some(pause.capture(
                    mutation::prepare_in(tx, command, source, fence, writing),
                    fence,
                )?);
                receipts::insert(tx, &identity, fence)
            },
            #[cfg(test)]
            fault,
        );
        pause.finish(
            executed.map(|_| result.expect("successful preparation stores its immutable record")),
        )
    }
}

#[cfg(test)]
#[path = "writing_prepare/tests.rs"]
mod tests;
