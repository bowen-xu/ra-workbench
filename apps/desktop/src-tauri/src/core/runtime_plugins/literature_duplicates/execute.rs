use super::DuplicateResources;
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction},
    runtime_plugins::native::{NativeHostError, NativeProfile, NativeResult},
};
use research_workbench_native_sdk::duplicate_review::DuplicateDecisionRequestV1;
use serde::Serialize;

/// Observations of this statement, not a durable receipt or a rollback claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum DecisionSqlState {
    NotDispatched,
    Returned {
        affected_rows: u64,
        observed_autocommit: bool,
    },
    Failed {
        observed_autocommit: bool,
    },
    Unknown,
}

pub(crate) fn execute_decision(
    guard: &mut DatabaseGuard<'_>,
    resources: &DuplicateResources,
    request: &DuplicateDecisionRequestV1,
) -> NativeResult<u64> {
    guarded_transaction::run(guard, |db| {
        if !db.is_autocommit() {
            let mut error = unsettled();
            error.duplicate_sql = Some(DecisionSqlState::NotDispatched);
            return (Err(error), true);
        }
        let result = resources.decide(db, request);
        let observed_autocommit = db.is_autocommit();
        let result = match result {
            Ok(affected_rows) if observed_autocommit => Ok(affected_rows),
            Ok(affected_rows) => {
                let mut error = unsettled();
                error.duplicate_sql = Some(DecisionSqlState::Returned {
                    affected_rows,
                    observed_autocommit,
                });
                Err(error)
            }
            Err(cause) => {
                let state = match cause.phase {
                    "execute" => DecisionSqlState::Failed {
                        observed_autocommit,
                    },
                    "validate" | "prepare" | "parameters" => DecisionSqlState::NotDispatched,
                    _ => DecisionSqlState::Unknown,
                };
                let mut error = NativeHostError::for_profile(
                    NativeProfile::NoteTraceDecision,
                    "storage_error",
                    format!("{}：{}", cause.message, cause.safe_cause),
                );
                error.data_error = Some(cause);
                error.duplicate_sql = Some(state);
                Err(error)
            }
        };
        // FAIL may leave committed trigger effects even when execute returned
        // Err. Never add rollback, replay or an unknown fence solely for Err.
        (result, !observed_autocommit)
    })
}

fn unsettled() -> Box<NativeHostError> {
    NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "storage_error",
        "重复项审核连接尚未结算，禁止自动重试；请重开应用后核对。",
    )
}
