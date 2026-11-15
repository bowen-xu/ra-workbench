//! Keep the data owner fenced through the complete queue admission transaction.
use super::*;
use crate::data_interface::DatabaseGuard;
use rusqlite::{DropBehavior, Transaction, TransactionBehavior};

fn unknown(
    record: &SubmissionRecord,
    phase: &'static str,
    cause: &'static str,
    error: Option<&rusqlite::Error>,
) -> Error {
    let (sqlite_code, sqlite_extended_code) = match error {
        Some(rusqlite::Error::SqliteFailure(code, _)) => {
            (Some(format!("{:?}", code.code)), Some(code.extended_code))
        }
        _ => (None, None),
    };
    Error::QueueAdmissionUnknown {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        phase,
        safe_cause: cause,
        sqlite_code,
        sqlite_extended_code,
    }
}

pub(crate) fn admit_managed(
    guard: &mut DatabaseGuard<'_>,
    record: &SubmissionRecord,
    confirmation: Option<Confirmation>,
    resume: bool,
) -> Result<(), Error> {
    let restored = restore_preview(guard, record.clone())?;
    let confirmation = match (restored.queue, confirmation) {
        (None, None) => return Ok(()),
        (Some(saved), Some(current)) if saved.input_id == current.input_id => current,
        _ => return Err(Error::ConfirmationMismatch),
    };
    let fence = guard.fence().ok_or(Error::Unavailable)?;
    let request = crate::agent::context_session::start::Request {
        conversation_id: record.conversation_id,
        request_id: record.request_id,
        context_snapshot_id: record.context_snapshot_id,
        queue: Some(confirmation),
        resume_queue: resume,
    };
    crate::data_interface::guarded_transaction::run(guard, |db| {
        let mut tx = match Transaction::new_unchecked(db, TransactionBehavior::Immediate) {
            Ok(tx) => tx,
            Err(error) => {
                let open = !db.is_autocommit();
                return (
                    Err(if open {
                        unknown(
                            record,
                            "begin",
                            "sql_transaction_remains_open",
                            Some(&error),
                        )
                    } else {
                        Error::Unavailable
                    }),
                    open,
                );
            }
        };
        tx.set_drop_behavior(DropBehavior::Ignore);
        let mut corrupt = false;
        let result = (|| {
            let has_receipt =
                crate::data_interface::check_context_queue_admission(&tx, fence, &request)
                    .map_err(|cause| match cause {
                        crate::data_interface::SqlImmediateFailure::Conflict(_) => {
                            Error::ConfirmationMismatch
                        }
                        crate::data_interface::SqlImmediateFailure::Corrupt(_) => {
                            corrupt = true;
                            Error::Unavailable
                        }
                        _ => Error::Unavailable,
                    })?;
            let has_business = crate::data_interface::has_legacy_admission(&tx, &request)
                .map_err(|_| Error::Unavailable)?;
            if has_receipt != has_business {
                corrupt = has_receipt;
                return Err(Error::Unavailable);
            }
            input_queue::admit_in_tx(
                &tx,
                record.conversation_id,
                record.request_id,
                confirmation.expected_revision,
                confirmation.input_id,
                record.input.clone(),
                &[],
                resume,
                |db| authorize(db, record.conversation_id),
            )
            .map_err(super::admission_error)?;
            if !has_receipt {
                crate::data_interface::insert_context_queue_admission(&tx, fence, &request)
                    .map_err(|_| Error::Unavailable)?;
            }
            Ok(())
        })();
        match result {
            Err(error) => {
                let rollback = tx.rollback();
                let unknown = rollback.is_err() || !db.is_autocommit();
                let result = if unknown {
                    Err(self::unknown(
                        record,
                        "rollback",
                        if rollback.is_err() {
                            "sql_rollback_failed"
                        } else {
                            "sql_transaction_remains_open"
                        },
                        rollback.as_ref().err(),
                    ))
                } else {
                    Err(error)
                };
                (result, unknown || corrupt)
            }
            Ok(_) => {
                if let Err(error) = tx.commit() {
                    if !db.is_autocommit() {
                        let _ = db.execute_batch("rollback");
                    }
                    return (
                        Err(unknown(
                            record,
                            "commit",
                            "sql_commit_outcome_unknown",
                            Some(&error),
                        )),
                        true,
                    );
                }
                if !db.is_autocommit() {
                    return (
                        Err(unknown(
                            record,
                            "commit",
                            "sql_transaction_remains_open",
                            None,
                        )),
                        true,
                    );
                }
                (Ok(()), false)
            }
        }
    })
}
