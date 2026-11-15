//! Preview cancellation shares the journal admission and SQL owner.
use super::*;
use crate::agent::{
    repository::{input_queue, submissions},
    writing_submission,
};
use research_agent_core::{
    conversation::CatalogError, input_queue::InputQueue, submission::SubmissionError,
};
use rusqlite::Connection;
#[path = "preview_cancel/types.rs"]
mod types;
use types::identity;
pub(crate) use types::{CancelPreviewIntent, QueuePreviewCancelStatus, QueuePreviewRetirement};
#[path = "preview_cancel/retirement.rs"]
mod retirement;
pub(super) use retirement::read_retirement_in;
#[path = "preview_cancel/compact.rs"]
mod compact;
#[path = "preview_cancel/status.rs"]
mod status;

fn error(
    command: &CancelPreviewIntent,
    fence: ReadFence,
    phase: &'static str,
    cause: SqlMutationError,
) -> SqlImmediateFailure {
    match cause {
        SqlMutationError::Sql(cause) => receipts::storage(command.request_id, fence, phase, &cause),
        SqlMutationError::Domain(CatalogError::Corrupt) => {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                command.request_id,
                fence,
                phase,
                "queue_cancel_state_corrupt",
            )))
        }
        SqlMutationError::Domain(cause) => SqlImmediateFailure::Domain(cause),
    }
}
fn writing_error(cause: writing_submission::WritingSubmissionError) -> SqlMutationError {
    use writing_submission::WritingSubmissionError as E;
    match cause {
        E::Unbound | E::Input(SubmissionError::Conflict) => CatalogError::Conflict,
        E::Input(SubmissionError::Corrupt) => CatalogError::Corrupt,
        E::Input(SubmissionError::InvalidInput) => CatalogError::InvalidInput,
        _ => CatalogError::Unavailable,
    }
    .into()
}
fn frozen_scope(db: &Connection, command: &CancelPreviewIntent) -> Result<bool, SqlMutationError> {
    let record = submissions::read(db, command.conversation_id, command.preview_request_id)
        .map_err(|cause| match cause {
            SubmissionError::Corrupt => CatalogError::Corrupt,
            _ => CatalogError::Unavailable,
        })?;
    let Some(record) = record else {
        return Ok(false);
    };
    writing_submission::validate_scope(&record, command.manuscript_id).map_err(writing_error)?;
    if writing_submission::read_compaction_revision_in(
        db,
        command.conversation_id,
        command.preview_request_id,
    )
    .map_err(writing_error)?
    .is_some()
    {
        return compact::frozen_scope(db, command);
    }
    Ok(true)
}
fn business(
    db: &Connection,
    command: &CancelPreviewIntent,
) -> Result<Option<InputQueue>, SqlMutationError> {
    input_queue::read_cancel_preview_receipt(
        db,
        command.conversation_id,
        command.request_id,
        command.expected_queue_revision,
        command.input_id,
        command.preview_request_id,
    )
}
impl Database {
    /// Exact legacy business receipts may acquire their first typed proof only
    /// under write admission after full original command and manuscript checks.
    pub(crate) fn cancel_queue_preview(
        &self,
        command: &CancelPreviewIntent,
    ) -> Result<(InputQueue, SqlImmediateReceipt), SqlImmediateFailure> {
        self.cancel_queue_preview_owned(
            command,
            #[cfg(test)]
            Fault::None,
        )
    }
    fn cancel_queue_preview_owned(
        &self,
        command: &CancelPreviewIntent,
        #[cfg(test)] fault: Fault,
    ) -> Result<(InputQueue, SqlImmediateReceipt), SqlImmediateFailure> {
        let identity = identity(command)?;
        let mut queue = None;
        let receipt = self.execute_sql(
            command.request_id,
            |tx, fence| {
                receipts::check_state(tx, command.request_id, fence, "queue-cancel")?;
                receipts::check_cross_kind(tx, command.request_id, fence, "queue-cancel")?;
                let receipt = receipts::lookup(tx, &identity, fence, "queue-cancel")?;
                writing_submission::check_binding(
                    tx,
                    command.conversation_id,
                    command.manuscript_id,
                )
                .map_err(|cause| {
                    error(
                        command,
                        fence,
                        "queue-cancel-authorize",
                        match cause {
                            writing_submission::WritingSubmissionError::Unbound => {
                                CatalogError::NotFound.into()
                            }
                            cause => writing_error(cause),
                        },
                    )
                })?;
                if !frozen_scope(tx, command)
                    .map_err(|cause| error(command, fence, "queue-cancel-scope", cause))?
                {
                    return Err(SqlImmediateFailure::Domain(CatalogError::Conflict));
                }
                let compact = compact::check_prepare_proof(tx, command, fence)?;
                let saved = business(tx, command)
                    .map_err(|cause| error(command, fence, "queue-cancel-replay", cause))?;
                // A legacy compact business row never proves a safe retirement.
                // Only this facade can create the paired cancellation proof.
                if compact && saved.is_some() && receipt.is_none() {
                    return Err(SqlImmediateFailure::Domain(CatalogError::Conflict));
                }
                if let Some(receipt) = receipt {
                    queue = Some(saved.ok_or_else(|| {
                        SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                            command.request_id,
                            fence,
                            "queue-cancel-replay",
                            "queue_cancel_business_receipt_missing",
                        )))
                    })?);
                    return Ok(receipt);
                }
                queue = Some(match saved {
                    Some(saved) => saved,
                    None => compact::cancel_in(tx, command)
                        .map_err(|cause| error(command, fence, "queue-cancel", cause))?,
                });
                receipts::insert(tx, &identity, fence)
            },
            #[cfg(test)]
            fault,
        )?;
        // The private transaction shell cannot succeed without its body result.
        Ok((
            queue.expect("successful cancel transaction sets its queue"),
            receipt,
        ))
    }
}

#[cfg(test)]
#[path = "preview_cancel/tests.rs"]
mod tests;
