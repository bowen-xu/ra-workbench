//! Typed SQL registrations under the journal owner's admission fence.
use super::Database;
#[path = "sql_immediate/writing_source.rs"]
mod writing_source;
#[cfg(test)]
#[path = "sql_immediate/writing_source_tests.rs"]
mod writing_source_tests;
use research_data_runtime::{CoordinatorError, ReadFence};
use rusqlite::{DropBehavior, Transaction, TransactionBehavior};
use uuid::Uuid;

#[path = "sql_immediate/archive.rs"]
mod archive;
#[path = "sql_immediate/child_cleanup.rs"]
mod child_cleanup;
#[path = "sql_immediate/create_bound.rs"]
mod create_bound;
pub(crate) use child_cleanup::{ChildCleanupIntent, ChildCleanupPrepared};
#[path = "sql_immediate/child_submission.rs"]
mod child_submission;
pub(crate) use child_submission::ChildSubmissionInspection;
#[path = "sql_immediate/child_submission_write.rs"]
mod child_submission_write;
pub(crate) use child_submission_write::{ChildSubmissionIntent, ChildSubmissionPrepared};
#[path = "sql_immediate/deletion.rs"]
mod deletion;
#[path = "sql_immediate/epoch.rs"]
mod epoch;
#[path = "sql_immediate/fork.rs"]
mod fork;
pub(crate) use deletion::DeletionFailure;
#[path = "sql_immediate/preview_cancel.rs"]
mod preview_cancel;
#[path = "sql_immediate/queue_edit.rs"]
mod queue_edit;
#[path = "sql_immediate/steering_handoff.rs"]
mod steering_handoff;
#[path = "sql_immediate/steering_handoff_cancel.rs"]
mod steering_handoff_cancel;
#[path = "sql_immediate/steering_handoff_finish.rs"]
mod steering_handoff_finish;
#[path = "sql_immediate/steering_handoff_release.rs"]
mod steering_handoff_release;
#[path = "sql_immediate/steering_handoff_status.rs"]
mod steering_handoff_status;
pub(crate) use queue_edit::{
    QUEUE_EDIT_FAMILY, QueueEditFailure, QueueEditIntent, QueueEditOutcome, QueueEditScope,
    QueueEditStatus,
};
pub(crate) use steering_handoff::{
    SteeringHandoffHistory, SteeringHandoffIntent, SteeringHandoffPrepared,
};
pub(crate) use steering_handoff_finish::FinishSteeringHandoff;
pub(crate) use steering_handoff_status::SteeringHandoffResolution;
#[path = "sql_immediate/context_configuration.rs"]
mod context_configuration;
#[path = "sql_immediate/context_prepare.rs"]
mod context_prepare;
pub(crate) use context_configuration::{
    ContextConfigurationEnvironmentRequest, ContextConfigurationFailure,
    ContextConfigurationIntent, ContextConfigurationSeedIntent,
};
#[path = "sql_immediate/context_queue_admission.rs"]
mod context_queue_admission;
#[path = "sql_immediate/context_start_status.rs"]
mod context_start_status;
pub(crate) use context_queue_admission::{
    check_context_queue_admission, has_legacy_admission, insert_context_queue_admission,
};
#[path = "sql_immediate/task_settlement.rs"]
mod task_settlement;
pub(crate) use context_prepare::{
    ContextPrepareIntent, ContextPrepareScopeFailure, ContextPrepareSource, ContextPrepareStatus,
};
pub(crate) use context_start_status::ContextStartInspection;
pub(crate) use task_settlement::TaskSettlement;
#[path = "sql_immediate/paper_prepare.rs"]
mod paper_prepare;
#[path = "sql_immediate/preview_prepare.rs"]
mod preview_prepare;
#[path = "sql_immediate/writing_prepare.rs"]
mod writing_prepare;
pub(crate) use paper_prepare::{
    PAPER_PREPARE_FAMILY, PaperPrepareIntent, PaperPrepareSource, PaperPrepareStatus,
};
#[path = "sql_immediate/paper_start_status.rs"]
mod paper_start_status;
pub(crate) use paper_start_status::PaperStartInspection;
pub(crate) use writing_prepare::{
    WRITING_PREPARE_FAMILY, WritingPrepareIntent, WritingPrepareSource, WritingPrepareStatus,
};
#[path = "sql_immediate/queued_start_status.rs"]
mod queued_start_status;
pub(crate) use queued_start_status::QueueStartInspection;
#[path = "sql_immediate/writing_start_status.rs"]
mod writing_start_status;
pub(crate) use writing_start_status::WritingStartInspection;
#[path = "sql_immediate/queued_compaction.rs"]
mod queued_compaction;
pub(crate) use preview_cancel::{
    CancelPreviewIntent, QueuePreviewCancelStatus, QueuePreviewRetirement,
};
pub(crate) use preview_prepare::{
    QueuePreviewPrepareIntent, QueuePreviewPrepareSource, QueuePreviewPrepareStatus,
    QueuePreviewPrepared,
};
#[path = "sql_immediate/receipts.rs"]
mod receipts;
#[path = "sql_immediate/rename.rs"]
mod rename;
#[path = "sql_immediate/types.rs"]
mod types;
use types::ReceiptIdentity;
pub(crate) use types::{
    AuthorizeRollback, SqlAuthorizationError, SqlImmediateDiagnostic, SqlImmediateFailure,
    SqlImmediateReceipt, SqlImmediateStatus, SqlMutationError,
};

impl Database {
    pub(crate) fn authorize_rollback(
        &self,
        command: &AuthorizeRollback,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        self.authorize_rollback_owned(
            command,
            #[cfg(test)]
            Fault::None,
        )
    }

    fn authorize_rollback_owned(
        &self,
        command: &AuthorizeRollback,
        #[cfg(test)] fault: Fault,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let fingerprint = command.fingerprint()?;
        self.execute_sql(
            command.operation_id,
            |tx, fence| receipts::register(tx, command, fence, fingerprint),
            #[cfg(test)]
            fault,
        )
    }

    // The transaction shell is private: typed operations supply their audited
    // transaction body without exposing SQL or a general closure API to callers.
    fn execute_sql(
        &self,
        operation_id: Uuid,
        register: impl FnOnce(
            &Transaction<'_>,
            ReadFence,
        ) -> Result<SqlImmediateReceipt, SqlImmediateFailure>,
        #[cfg(test)] fault: Fault,
    ) -> Result<SqlImmediateReceipt, SqlImmediateFailure> {
        let owner = self.owner.as_ref().ok_or_else(|| unmanaged(operation_id))?;
        let mut admission = owner.materialize_guard().map_err(|error| {
            let fence = owner.inspection_guard().ok().map(|guard| {
                let state = guard.snapshot();
                ReadFence {
                    epoch: state.epoch,
                    position: state.materialized_position,
                }
            });
            admission_error(operation_id, error, fence)
        })?;
        let fence = admission.fence();
        let db = self.raw.lock().map_err(|_| {
            admission.block();
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                operation_id,
                fence,
                "sql-lock",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            admission.block();
            return Err(SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                operation_id,
                fence,
                "begin",
                "sql_transaction_already_open",
            ))));
        }
        // SQL is private and autocommit was checked while holding its mutex.
        // Shared borrowing permits explicit cleanup inspection after a failure.
        let mut tx = match Transaction::new_unchecked(&db, TransactionBehavior::Immediate) {
            Ok(tx) => tx,
            Err(error) => {
                let detail =
                    diagnostic(operation_id, fence, "begin", "sql_begin_failed").sqlite(&error);
                return Err(if db.is_autocommit() {
                    SqlImmediateFailure::Unavailable(Box::new(detail))
                } else {
                    admission.block();
                    SqlImmediateFailure::Unknown(Box::new(detail))
                });
            }
        };
        // A consuming commit/rollback must not hide a second cleanup attempt in
        // Drop. Every expected exit below owns and checks its explicit cleanup.
        tx.set_drop_behavior(DropBehavior::Ignore);
        let prepared =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| register(&tx, fence))) {
                Ok(prepared) => prepared,
                Err(payload) => {
                    admission.block();
                    std::panic::resume_unwind(payload);
                }
            };
        let receipt = match prepared {
            Ok(receipt) => receipt,
            Err(failure) => {
                let rollback = tx.rollback();
                if let Err(error) = rollback {
                    admission.block();
                    return Err(SqlImmediateFailure::Unknown(Box::new(
                        diagnostic(operation_id, fence, "rollback", "sql_rollback_failed")
                            .sqlite(&error),
                    )));
                }
                if !db.is_autocommit() {
                    admission.block();
                    return Err(SqlImmediateFailure::Unknown(Box::new(diagnostic(
                        operation_id,
                        fence,
                        "rollback",
                        "sql_transaction_remains_open",
                    ))));
                }
                if matches!(failure, SqlImmediateFailure::Corrupt(_)) {
                    admission.block();
                }
                return Err(failure);
            }
        };
        #[cfg(test)]
        if let Fault::Pause { entered, proceed } = &fault {
            entered.send(()).unwrap();
            proceed.recv().unwrap();
        }
        #[cfg(test)]
        if matches!(&fault, Fault::BeforeCommit | Fault::Rollback) {
            drop(tx);
            if !matches!(&fault, Fault::Rollback) {
                db.execute_batch("rollback;").unwrap();
            }
            admission.block();
            return Err(SqlImmediateFailure::Unknown(Box::new(diagnostic(
                operation_id,
                fence,
                if matches!(&fault, Fault::Rollback) {
                    "rollback"
                } else {
                    "commit"
                },
                if matches!(&fault, Fault::Rollback) {
                    "injected_rollback_failure"
                } else {
                    "injected_commit_outcome_unknown"
                },
            ))));
        }
        let commit = tx.commit();
        if let Err(error) = commit {
            admission.block();
            let mut detail =
                diagnostic(operation_id, fence, "commit", "sql_commit_outcome_unknown")
                    .sqlite(&error);
            if !db.is_autocommit() {
                // Preserve the original commit's codes if cleanup also fails;
                // the stable classification states that recovery is required.
                if db.execute_batch("rollback;").is_err() || !db.is_autocommit() {
                    detail.safe_cause = "sql_commit_unknown_cleanup_failed";
                }
            }
            return Err(SqlImmediateFailure::Unknown(Box::new(detail)));
        }
        #[cfg(test)]
        if matches!(&fault, Fault::AfterCommit) {
            admission.block();
            return Err(SqlImmediateFailure::Unknown(Box::new(diagnostic(
                operation_id,
                fence,
                "commit",
                "injected_commit_acknowledgement_lost",
            ))));
        }
        if !db.is_autocommit() {
            admission.block();
            return Err(SqlImmediateFailure::Unknown(Box::new(diagnostic(
                operation_id,
                fence,
                "commit",
                "sql_commit_left_transaction_open",
            ))));
        }
        Ok(receipt)
    }

    /// Receipt inspection does not establish current rollback permission.
    /// It never materializes pending data or repairs/clears an uncertain owner.
    pub(crate) fn rollback_authorization_status(
        &self,
        command: &AuthorizeRollback,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        self.sql_operation_status(&ReceiptIdentity {
            operation_id: command.operation_id,
            kind: types::KIND,
            version: types::VERSION,
            fingerprint: command.fingerprint()?,
        })
    }

    fn sql_operation_status(
        &self,
        identity: &ReceiptIdentity,
    ) -> Result<SqlImmediateStatus, SqlImmediateFailure> {
        let operation_id = identity.operation_id;
        let owner = self.owner.as_ref().ok_or_else(|| unmanaged(operation_id))?;
        let inspection = owner
            .inspection_guard()
            .map_err(|error| admission_error(operation_id, error, None))?;
        let snapshot = inspection.snapshot();
        let fence = ReadFence {
            epoch: snapshot.epoch,
            position: snapshot.materialized_position,
        };
        let db = self.raw.lock().map_err(|_| {
            SqlImmediateFailure::RecoveryRequired(Box::new(diagnostic(
                operation_id,
                fence,
                "status",
                "sql_mutex_poisoned",
            )))
        })?;
        if !db.is_autocommit() {
            return Ok(SqlImmediateStatus::Unconfirmed {
                epoch: fence.epoch,
                sql_fence: fence,
                blocked: snapshot.blocked,
                closed: snapshot.closed,
                reason: "sql_transaction_open",
            });
        }
        // A failed projection commit acknowledgement can leave SQL ahead of
        // the cached materialized head. Verify the actual SQL epoch/head; the
        // cache itself is not proof of a receipt's commit or non-commit.
        let fence = receipts::status_fence(&db, operation_id, fence, snapshot.accepted_position)?;
        receipts::check_cross_kind(&db, operation_id, fence, "status")?;
        if let Some(receipt) = receipts::lookup(&db, identity, fence, "status")? {
            return Ok(SqlImmediateStatus::Committed(receipt));
        }
        if snapshot.blocked {
            Ok(SqlImmediateStatus::Unconfirmed {
                epoch: fence.epoch,
                sql_fence: fence,
                blocked: true,
                closed: snapshot.closed,
                reason: "owner_blocked_receipt_unconfirmed",
            })
        } else {
            Ok(SqlImmediateStatus::NoSqlReceipt {
                epoch: fence.epoch,
                sql_fence: fence,
                closed: snapshot.closed,
            })
        }
    }
}

fn unmanaged(operation_id: Uuid) -> SqlImmediateFailure {
    SqlImmediateFailure::Unmanaged(Box::new(SqlImmediateDiagnostic::for_operation(
        operation_id,
        None,
        "admission",
        "managed_owner_required",
    )))
}

fn diagnostic(
    operation_id: Uuid,
    fence: ReadFence,
    phase: &'static str,
    cause: &'static str,
) -> SqlImmediateDiagnostic {
    SqlImmediateDiagnostic::for_operation(operation_id, Some(fence), phase, cause)
}

fn admission_error(
    operation_id: Uuid,
    error: CoordinatorError,
    fence: Option<ReadFence>,
) -> SqlImmediateFailure {
    let cause = match error {
        CoordinatorError::RecoveryRequired => "owner_recovery_required",
        CoordinatorError::Closed => "owner_closed",
        CoordinatorError::Projector { .. } => "pending_projection_failed",
        CoordinatorError::CommitUnknown { .. } => "prior_log_commit_unknown",
        CoordinatorError::AcceptedIncomplete { .. } => "prior_log_accept_incomplete",
        _ => "owner_admission_failed",
    };
    SqlImmediateFailure::RecoveryRequired(Box::new(SqlImmediateDiagnostic::for_operation(
        operation_id,
        fence,
        "admission",
        cause,
    )))
}

#[cfg(test)]
enum Fault {
    None,
    BeforeCommit,
    AfterCommit,
    Rollback,
    Pause {
        entered: std::sync::mpsc::Sender<()>,
        proceed: std::sync::mpsc::Receiver<()>,
    },
}

#[cfg(test)]
#[path = "sql_immediate/tests.rs"]
mod tests;

#[path = "sql_immediate/writing_context_read.rs"]
mod writing_context_read;
pub(crate) use writing_context_read::ContextReadMode;
