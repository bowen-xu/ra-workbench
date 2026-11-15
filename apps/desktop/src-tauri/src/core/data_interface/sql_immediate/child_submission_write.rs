//! Atomically freeze a child submission and its operation receipt.
use super::{
    Database, ReadFence, ReceiptIdentity, SqlImmediateFailure, Transaction, diagnostic, receipts,
};
use crate::agent::repository::{SqlRepositoryError, submissions};
use research_agent_core::{
    conversation::{BindingKind, CatalogError, ConversationBinding, child_submission_request_id},
    submission::{SubmissionError, SubmissionInput, SubmissionRecord},
};
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const KIND: &str = "agent.child.submission";

#[derive(Clone)]
pub(crate) struct ChildSubmissionIntent {
    pub parent: Uuid,
    pub child: Uuid,
    pub request: Uuid,
    pub binding: ConversationBinding,
    pub input: SubmissionInput,
}

#[derive(Debug)]
pub(crate) struct ChildSubmissionPrepared {
    pub record: SubmissionRecord,
    pub replayed: bool,
}

impl ChildSubmissionIntent {
    pub(crate) fn operation(&self) -> Uuid {
        Uuid::new_v5(&self.child, b"agent.child.submission.persist:v1")
    }

    fn identity(&self) -> Result<ReceiptIdentity, SqlImmediateFailure> {
        if self.parent.is_nil()
            || self.child.is_nil()
            || self.request != child_submission_request_id(self.child)
            || self.input.validate().is_err()
            || self.binding.resource_id.trim().is_empty()
            || self.binding.resource_id.chars().count() > 256
            || self.binding.resource_id.contains('\0')
        {
            return Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput));
        }
        #[derive(Serialize)]
        struct Frozen<'a> {
            kind: &'static str,
            version: u32,
            parent: Uuid,
            child: Uuid,
            request: Uuid,
            binding: &'a ConversationBinding,
            input: &'a SubmissionInput,
        }
        let bytes = serde_json::to_vec(&Frozen {
            kind: KIND,
            version: 1,
            parent: self.parent,
            child: self.child,
            request: self.request,
            binding: &self.binding,
            input: &self.input,
        })
        .map_err(|_| SqlImmediateFailure::Domain(CatalogError::InvalidInput))?;
        Ok(ReceiptIdentity {
            operation_id: self.operation(),
            kind: KIND,
            version: 1,
            fingerprint: Sha256::digest(bytes).into(),
        })
    }
}

fn repository_error(
    operation: Uuid,
    fence: ReadFence,
    error: SqlRepositoryError<SubmissionError>,
) -> SqlImmediateFailure {
    match error {
        SqlRepositoryError::Sql(error) => {
            receipts::storage(operation, fence, "child-submission-write", &error)
        }
        SqlRepositoryError::Domain(error) => {
            let detail = Box::new(diagnostic(
                operation,
                fence,
                "child-submission-write",
                "child_submission_rejected",
            ));
            match error {
                SubmissionError::InvalidInput => SqlImmediateFailure::Invalid(detail),
                SubmissionError::Conflict => SqlImmediateFailure::Conflict(detail),
                SubmissionError::Corrupt => SqlImmediateFailure::Corrupt(detail),
                SubmissionError::Unavailable => SqlImmediateFailure::Unavailable(detail),
                SubmissionError::NotFound => SqlImmediateFailure::Domain(CatalogError::NotFound),
            }
        }
    }
}

fn bound(
    tx: &Transaction<'_>,
    intent: &ChildSubmissionIntent,
    fence: ReadFence,
) -> Result<(), SqlImmediateFailure> {
    let target = research_agent_core::conversation::CreateBoundConversation {
        operation_id: intent.operation(),
        id: research_agent_core::conversation::ConversationId::from_uuid(intent.child),
        binding: intent.binding.clone(),
    };
    crate::agent::repository::check_create_bound_target_in(tx, &target).map_err(
        |error| match error {
            super::SqlMutationError::Sql(error) => receipts::storage(
                intent.operation(),
                fence,
                "child-submission-binding",
                &error,
            ),
            super::SqlMutationError::Domain(_) => {
                SqlImmediateFailure::Conflict(Box::new(diagnostic(
                    intent.operation(),
                    fence,
                    "child-submission-binding",
                    "child_submission_target_unavailable",
                )))
            }
        },
    )?;
    let kind = match intent.binding.kind {
        BindingKind::Manuscript => "manuscript",
        BindingKind::Paper => "paper",
        BindingKind::Note => "note",
        BindingKind::ReadingNote => "readingNote",
    };
    for id in [intent.parent, intent.child] {
        let found: bool = tx.query_row("select exists(select 1 from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id where b.conversation_id=?1 and b.resource_kind=?2 and b.resource_id=?3 and c.archived=0)",
            params![id.to_string(), kind, intent.binding.resource_id], |row| row.get(0))
            .map_err(|error| receipts::storage(intent.operation(), fence, "child-submission-binding", &error))?;
        if !found {
            return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                intent.operation(),
                fence,
                "child-submission-binding",
                "child_submission_binding_changed",
            ))));
        }
    }
    Ok(())
}

impl Database {
    pub(crate) fn prepare_child_submission(
        &self,
        intent: &ChildSubmissionIntent,
    ) -> Result<ChildSubmissionPrepared, SqlImmediateFailure> {
        self.prepare_child_submission_owned(
            intent,
            #[cfg(test)]
            false,
            #[cfg(test)]
            super::Fault::None,
        )
    }

    #[cfg(all(test, debug_assertions, feature = "development-data"))]
    pub(crate) fn preview_child_submission_failure(
        &self,
        intent: &ChildSubmissionIntent,
    ) -> Result<ChildSubmissionPrepared, SqlImmediateFailure> {
        self.prepare_child_submission_owned(
            intent,
            true,
            #[cfg(test)]
            super::Fault::None,
        )
    }

    fn prepare_child_submission_owned(
        &self,
        intent: &ChildSubmissionIntent,
        #[cfg(test)] preview_failure: bool,
        #[cfg(test)] fault: super::Fault,
    ) -> Result<ChildSubmissionPrepared, SqlImmediateFailure> {
        let identity = intent.identity()?;
        let mut result = None;
        let receipt = self.execute_sql(
            identity.operation_id,
            |tx, fence| {
                receipts::check_state(tx, identity.operation_id, fence, "child-submission-write")?;
                receipts::check_cross_kind(
                    tx,
                    identity.operation_id,
                    fence,
                    "child-submission-write",
                )?;
                let cleanup_pending: bool = tx.query_row(
                    "select exists(select 1 from agent_child_cleanup_intents where child_id=?1)",
                    [intent.child.to_string()],
                    |row| row.get(0),
                ).map_err(|error| receipts::storage(identity.operation_id, fence, "child-submission-cleanup-check", &error))?;
                if cleanup_pending {
                    return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                        identity.operation_id, fence, "child-submission-cleanup-check", "child_cleanup_already_prepared",
                    ))));
                }
                let receipt = receipts::lookup(tx, &identity, fence, "child-submission-write")?;
                bound(tx, intent, fence)?;
                let existing = submissions::read_sql(tx, intent.child, intent.request)
                    .map_err(|error| repository_error(identity.operation_id, fence, error))?;
                match (receipt, existing) {
                    (Some(receipt), Some(record)) if record.input == intent.input => {
                        result = Some(record);
                        return Ok(receipt);
                    }
                    (Some(_), _) => {
                        return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                            identity.operation_id,
                            fence,
                            "child-submission-write",
                            "child_submission_receipt_mismatch",
                        ))));
                    }
                    (None, Some(_)) => {
                        return Err(SqlImmediateFailure::Conflict(Box::new(diagnostic(
                            identity.operation_id,
                            fence,
                            "child-submission-write",
                            "legacy_child_submission_without_receipt",
                        ))));
                    }
                    (None, None) => {}
                }
                #[cfg(test)]
                if preview_failure {
                    return Err(SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                        identity.operation_id,
                        fence,
                        "child-submission-write",
                        "synthetic_preview_submission_failure",
                    ))));
                }
                result = Some(
                    submissions::insert_in_sql(
                        tx,
                        intent.child,
                        intent.request,
                        intent.input.clone(),
                    )
                    .map_err(|error| repository_error(identity.operation_id, fence, error))?,
                );
                receipts::insert(tx, &identity, fence)
            },
            #[cfg(test)]
            fault,
        )?;
        let record = result.ok_or_else(|| {
            SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                identity.operation_id,
                receipt.sql_fence,
                "child-submission-write",
                "child_submission_result_missing",
            )))
        })?;
        Ok(ChildSubmissionPrepared {
            record,
            replayed: receipt.replayed,
        })
    }
}

#[cfg(test)]
#[path = "child_submission_write_tests.rs"]
mod tests;
