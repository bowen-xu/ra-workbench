//! Private HBS validation suspension and settlement, never a native/service API.
use crate::{
    ai::{
        generation_policy::GenerationPolicySource,
        paper_reading_source::PaperReadingSource,
        structured_resource_validation::{
            Failure, FailureCause, Flow, StructuredPolicy, ValidationSources,
        },
    },
    runtime_plugins::native::SafeDatabaseError,
};
use research_workbench_native_sdk::review_selection::UnreviewedSelectionInputV1;
use research_workbench_native_sdk::structured_validation::StructuredValidationInputV1;
use rusqlite::{Connection, DropBehavior, Transaction};
use serde::Serialize;
use serde_json::Value;
use std::convert::Infallible;

type SourcesResult<'a, P> =
    Result<ValidationSources<'a, P>, Flow<String, <P as StructuredPolicy>::Pause>>;

pub(crate) trait HbsSources {
    type Policy: StructuredPolicy;
    fn policy_only(
        &mut self,
    ) -> Result<&mut Self::Policy, Flow<String, <Self::Policy as StructuredPolicy>::Pause>>;
    fn require(&mut self) -> SourcesResult<'_, Self::Policy>;
    /// Preserve the original static source gate; runtime validation stays lazy.
    fn eager_validation_sources(&self) -> bool {
        false
    }
    fn explicit_settlement(&self) -> bool {
        true
    }
}
impl<P: StructuredPolicy> HbsSources for ValidationSources<'_, P> {
    type Policy = P;
    fn policy_only(&mut self) -> Result<&mut P, Flow<String, P::Pause>> {
        Ok(&mut *self.policy)
    }
    fn require(&mut self) -> SourcesResult<'_, P> {
        Ok(ValidationSources {
            policy: &mut *self.policy,
            reading: self.reading,
        })
    }
}
pub(crate) struct StaticHbsSources {
    policy: Option<GenerationPolicySource>,
    reading: Option<PaperReadingSource>,
}
impl StaticHbsSources {
    pub(crate) fn new(
        policy: Option<GenerationPolicySource>,
        reading: Option<PaperReadingSource>,
    ) -> Self {
        Self { policy, reading }
    }
}
impl HbsSources for StaticHbsSources {
    type Policy = GenerationPolicySource;
    fn eager_validation_sources(&self) -> bool {
        true
    }
    fn policy_only(&mut self) -> Result<&mut Self::Policy, Flow<String, Infallible>> {
        self.policy.as_mut().ok_or_else(|| {
            Flow::original(
                "CAPABILITY_UNAVAILABLE: literature_library.generation_policy_source".into(),
            )
        })
    }
    fn require(&mut self) -> Result<ValidationSources<'_, Self::Policy>, Flow<String, Infallible>> {
        let policy = self.policy.as_mut().ok_or_else(|| {
            Flow::original(
                "CAPABILITY_UNAVAILABLE: literature_library.generation_policy_source".into(),
            )
        })?;
        let reading = self.reading.as_ref().ok_or_else(|| {
            Flow::original("CAPABILITY_UNAVAILABLE: literature_library.paper_reading_source".into())
        })?;
        Ok(ValidationSources { policy, reading })
    }
    fn explicit_settlement(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub(crate) enum HbsNeed {
    Sources,
    EditableTemplate {
        slot: usize,
        input: research_workbench_native_sdk::editable_template::EditableTemplateInputV1,
    },
    Structured {
        slot: usize,
        input: StructuredValidationInputV1,
    },
    Unreviewed {
        slot: usize,
        input: UnreviewedSelectionInputV1,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HbsOutcome {
    NotStarted,
    RolledBack,
    Committed,
    Unknown,
}
#[derive(Debug)]
pub(crate) struct BoundaryFailure<E = String> {
    pub(crate) failure: Failure<E>,
    pub(crate) phase: &'static str,
    pub(crate) outcome: HbsOutcome,
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}
impl<E> BoundaryFailure<E> {
    pub(crate) fn needs_recovery(&self) -> bool {
        self.outcome == HbsOutcome::Unknown
    }
}
#[derive(Debug)]
pub(crate) struct ConfirmedPause<P> {
    need: P,
    outcome: HbsOutcome,
}
impl<P> ConfirmedPause<P> {
    pub(crate) fn need(self) -> P {
        self.need
    }
    pub(crate) fn outcome(&self) -> HbsOutcome {
        self.outcome
    }
}
#[derive(Debug)]
pub(crate) enum BoundaryError<P, E = String> {
    Failure(BoundaryFailure<E>),
    Pause(ConfirmedPause<P>),
}
impl<E> BoundaryError<Infallible, E> {
    pub(crate) fn into_static(self) -> E {
        match self {
            Self::Failure(failure) => failure.failure.error,
            Self::Pause(pause) => match pause.need {},
        }
    }
}
pub(super) fn failure<P, E>(
    failure: Failure<E>,
    phase: &'static str,
    outcome: HbsOutcome,
) -> BoundaryError<P, E> {
    BoundaryError::Failure(BoundaryFailure {
        failure,
        phase,
        outcome,
        rollback_error: None,
    })
}

fn failure_phase<E>(failure: &Failure<E>, fallback: &'static str) -> &'static str {
    match &failure.cause {
        FailureCause::SourceDatabase(error) => match error.phase {
            "sources" | "source-read" | "validation" | "begin" | "read" | "write" | "receipt"
            | "commit" | "postcommit-read" | "rollback" | "execution" => error.phase,
            _ => fallback,
        },
        FailureCause::SourceData(_) | FailureCause::SourceReadUnavailable => "source-read",
        _ => fallback,
    }
}

// This value has no public constructor and cannot leave a synchronous owner as a
// pause until confirm checks the connection after the Transaction was consumed.
pub(crate) enum Settlement<P, T = Value, E = String> {
    Complete(T),
    Failure(BoundaryFailure<E>),
    Pause(P),
}
pub(super) fn settle<P>(
    tx: Transaction<'_>,
    result: Result<Value, Flow<String, P>>,
    phase: &'static str,
    explicit: bool,
    commit: bool,
) -> Settlement<P> {
    settle_with(
        tx,
        result,
        phase,
        explicit,
        commit,
        |error| error.to_string(),
        str::to_owned,
    )
}
pub(crate) fn settle_with<P, T, E>(
    mut tx: Transaction<'_>,
    result: Result<T, Flow<E, P>>,
    phase: &'static str,
    explicit: bool,
    commit: bool,
    sql: impl Fn(rusqlite::Error) -> E,
    unsettled: impl Fn(&'static str) -> E,
) -> Settlement<P, T, E> {
    let error = match result {
        Ok(value) => {
            if commit {
                if explicit {
                    tx.set_drop_behavior(DropBehavior::Ignore);
                }
                return match tx.commit() {
                    Ok(()) => Settlement::Complete(value),
                    Err(error) => {
                        let cause = FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(
                            "commit", &error,
                        )));
                        Settlement::Failure(BoundaryFailure {
                            failure: Failure {
                                error: sql(error),
                                cause,
                            },
                            phase: "commit",
                            outcome: HbsOutcome::Unknown,
                            rollback_error: None,
                        })
                    }
                };
            }
            if !explicit {
                drop(tx);
                return Settlement::Complete(value);
            }
            tx.set_drop_behavior(DropBehavior::Ignore);
            return match tx.rollback() {
                Ok(()) => Settlement::Complete(value),
                Err(error) => rollback_failed(
                    Failure::original(unsettled("MODEL_STORAGE_ERROR：只读事务回滚未确认")),
                    phase,
                    &error,
                ),
            };
        }
        Err(error) => error,
    };
    let phase = match &error {
        Flow::Fail(failure) => failure_phase(failure, phase),
        Flow::Need(_) => phase,
    };
    let error = match (explicit, error) {
        (false, Flow::Fail(failure)) => {
            // The original static path deliberately retains its original Drop policy.
            drop(tx);
            return Settlement::Failure(BoundaryFailure {
                failure,
                phase,
                outcome: HbsOutcome::Unknown,
                rollback_error: None,
            });
        }
        (_, error) => error,
    };
    tx.set_drop_behavior(DropBehavior::Ignore);
    let rollback = tx.rollback();
    match (error, rollback) {
        (Flow::Need(need), Ok(())) => Settlement::Pause(need),
        (Flow::Fail(failure), Ok(())) => Settlement::Failure(BoundaryFailure {
            failure,
            phase,
            outcome: HbsOutcome::RolledBack,
            rollback_error: None,
        }),
        (Flow::Need(_), Err(error)) => rollback_failed(
            Failure::original(unsettled("MODEL_STORAGE_ERROR：验证准备回滚未确认")),
            phase,
            &error,
        ),
        (Flow::Fail(failure), Err(error)) => rollback_failed(failure, phase, &error),
    }
}
fn rollback_failed<P, T, E>(
    failure: Failure<E>,
    phase: &'static str,
    error: &rusqlite::Error,
) -> Settlement<P, T, E> {
    Settlement::Failure(BoundaryFailure {
        failure,
        phase,
        outcome: HbsOutcome::Unknown,
        rollback_error: Some(Box::new(SafeDatabaseError::sql("rollback", error))),
    })
}
pub(super) fn confirm<P>(
    db: &Connection,
    settled: Settlement<P>,
    explicit: bool,
) -> Result<Value, BoundaryError<P>> {
    confirm_with(db, settled, explicit, str::to_owned)
}
pub(crate) fn confirm_with<P, T, E>(
    db: &Connection,
    settled: Settlement<P, T, E>,
    explicit: bool,
    unsettled: impl Fn(&'static str) -> E,
) -> Result<T, BoundaryError<P, E>> {
    let autocommit = db.is_autocommit();
    match settled {
        Settlement::Pause(need) if autocommit => Err(BoundaryError::Pause(ConfirmedPause {
            need,
            outcome: HbsOutcome::RolledBack,
        })),
        Settlement::Pause(_) => Err(failure(
            Failure::original(unsettled("MODEL_STORAGE_ERROR：验证准备回滚未确认")),
            "rollback",
            HbsOutcome::Unknown,
        )),
        Settlement::Complete(value) if !explicit || autocommit => Ok(value),
        Settlement::Complete(_) => Err(failure(
            Failure::original(unsettled("MODEL_STORAGE_ERROR：事务结算未确认")),
            "commit",
            HbsOutcome::Unknown,
        )),
        Settlement::Failure(mut value) => {
            if explicit && !autocommit {
                value.outcome = HbsOutcome::Unknown;
            }
            Err(BoundaryError::Failure(value))
        }
    }
}
pub(super) fn read_result<P>(
    db: &Connection,
    result: Result<Value, Flow<String, P>>,
    phase: &'static str,
) -> Result<Value, BoundaryError<P>> {
    match result {
        Ok(value) => Ok(value),
        Err(Flow::Fail(value)) => {
            let phase = failure_phase(&value, phase);
            Err(failure(value, phase, HbsOutcome::NotStarted))
        }
        Err(Flow::Need(need)) if db.is_autocommit() => Err(BoundaryError::Pause(ConfirmedPause {
            need,
            outcome: HbsOutcome::NotStarted,
        })),
        Err(Flow::Need(_)) => Err(failure(
            Failure::original("MODEL_STORAGE_ERROR：只读验证仍持有事务".into()),
            "execution",
            HbsOutcome::Unknown,
        )),
    }
}
