//! Reuse the existing transaction suspension, rollback and recovery barrier.
use crate::{
    ai::{
        error::AiError,
        structured_resource_validation::{FailureCause, Flow},
    },
    hbs_storage::boundary::{self, BoundaryError},
    runtime_plugins::native::{NativeHostError, NativeProfile, StorageMutationDiagnostic},
};
use rusqlite::{Connection, Transaction};

pub(super) fn settle<P, T>(
    tx: Transaction<'_>,
    result: Result<T, Flow<AiError, P>>,
    explicit: bool,
) -> boundary::Settlement<P, T, AiError> {
    boundary::settle_with(tx, result, "write", explicit, true, AiError::from, |_| {
        AiError::TemporaryUnavailable
    })
}
pub(super) fn finish<P, T>(
    db: &Connection,
    settled: boundary::Settlement<P, T, AiError>,
    explicit: bool,
    batch: &str,
) -> Result<T, Flow<AiError, P>> {
    match boundary::confirm_with(db, settled, explicit, |_| AiError::TemporaryUnavailable) {
        Ok(value) => Ok(value),
        Err(BoundaryError::Pause(pause)) => Err(Flow::Need(pause.need())),
        Err(BoundaryError::Failure(failure)) if explicit => {
            let unknown = failure.needs_recovery();
            let code = if unknown {
                "data_operation_outcome_unknown"
            } else {
                failure.failure.error.code()
            };
            let mut message = if unknown {
                format!(
                    "确认批次 {batch} 的保存结果尚未确认。阶段 {}。原错误码 {}。",
                    failure.phase,
                    failure.failure.error.code()
                )
            } else {
                failure.failure.error.user_message()
            };
            if let FailureCause::SourceDatabase(database) = &failure.failure.cause {
                message.push_str(&format!("。安全原因 {database}"));
            }
            let mut native = match failure.failure.error {
                AiError::NativeWorkflow { failure, .. } => failure,
                _ => NativeHostError::for_profile(NativeProfile::NoteTraceDecision, code, &message),
            };
            native.code = code.into();
            native.message = message;
            if let FailureCause::NativeRejection(origin) = &failure.failure.cause {
                native.package_key = Some(origin.package_key.clone());
                native.instance_id = Some(origin.instance_id.clone());
                native.request_id = Some(origin.native_request_id.clone());
            }
            native.paper_confirmation = Some(Box::new(StorageMutationDiagnostic {
                phase: failure.phase,
                outcome: failure.outcome,
                object_id: Some(batch.into()),
                revision: None,
                database_error: match failure.failure.cause {
                    FailureCause::SourceDatabase(cause) => Some(cause),
                    _ => None,
                },
                rollback_error: failure.rollback_error,
            }));
            Err(Flow::original(AiError::NativeWorkflow {
                code,
                failure: native,
            }))
        }
        Err(BoundaryError::Failure(failure)) => Err(Flow::Fail(failure.failure)),
    }
}
