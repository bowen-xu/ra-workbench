//! The original resource operations use the shared transaction settlement owner.
use super::super::{Error, Resource, ReviewDraft};
use crate::{
    ai::{
        generation_policy::GenerationPolicySource,
        structured_resource_validation::{Failure, FailureCause, Flow, StructuredPolicy},
    },
    hbs_storage::boundary::{self, BoundaryError, BoundaryFailure, HbsOutcome, Settlement},
    runtime_plugins::native::SafeDatabaseError,
};
use research_workbench_native_sdk::review_selection::UnreviewedSelectionInputV1;
use rusqlite::Transaction;

impl<P> From<Error> for Flow<Error, P> {
    fn from(error: Error) -> Self {
        Self::original(error)
    }
}
impl<P> From<rusqlite::Error> for Flow<Error, P> {
    fn from(error: rusqlite::Error) -> Self {
        Self::original(Error::Database(error))
    }
}

pub(crate) trait ReviewPolicy: StructuredPolicy {
    fn select_unreviewed(
        &mut self,
        input: UnreviewedSelectionInputV1,
    ) -> Result<Vec<String>, Self::Pause>;
}
impl ReviewPolicy for GenerationPolicySource {
    fn select_unreviewed(
        &mut self,
        input: UnreviewedSelectionInputV1,
    ) -> Result<Vec<String>, Self::Pause> {
        Ok((self.unreviewed_fields)(
            &input.pending_field_ids,
            &input.protected_field_ids,
            &input.reviewed_field_ids,
        ))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ReviewRoute {
    Static,
    Native,
}
impl ReviewRoute {
    pub(crate) fn explicit(self) -> bool {
        matches!(self, Self::Native)
    }
    pub(crate) fn review(self, resource: &Resource, input: &ReviewDraft) -> Result<(), Error> {
        if self.explicit()
            && (!matches!(resource.kind.as_str(), "portrait" | "structuredReading")
                || input.action != "keep"
                || input.scope.as_deref() != Some("unreviewed")
                || input.field_ids.is_some())
        {
            return Err(Error::Invalid(
                "原生审阅仅支持画像或结构化阅读的 Keep 未校验".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn history(self, resource: &Resource) -> Result<(), Error> {
        if self.explicit() && !matches!(resource.kind.as_str(), "portrait" | "structuredReading") {
            return Err(Error::Invalid("原生历史回放仅支持画像或结构化阅读".into()));
        }
        Ok(())
    }
}

fn project_failure(error: impl Into<Error>, phase: &'static str) -> Failure<Error> {
    let error = error.into();
    let cause = match &error {
        Error::Database(cause) => {
            FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(phase, cause)))
        }
        _ => FailureCause::Original,
    };
    Failure { error, cause }
}
pub(crate) fn before_transaction<P>(
    error: impl Into<Error>,
    phase: &'static str,
) -> BoundaryError<P, Error> {
    BoundaryError::Failure(BoundaryFailure {
        failure: project_failure(error, phase),
        phase,
        outcome: HbsOutcome::NotStarted,
        rollback_error: None,
    })
}
pub(crate) fn settle<P>(
    tx: Transaction<'_>,
    result: Result<(Resource, bool), Flow<Error, P>>,
    phase: &'static str,
    explicit: bool,
) -> Settlement<P, Resource, Error> {
    let result = result.map_err(|error| match error {
        Flow::Fail(mut failure) => {
            if matches!(failure.cause, FailureCause::Original)
                && let Error::Database(cause) = &failure.error
            {
                failure.cause =
                    FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(phase, cause)));
            }
            Flow::Fail(failure)
        }
        error => error,
    });
    let (result, commit) = match result {
        Ok((resource, commit)) => (Ok(resource), commit),
        Err(error) => (Err(error), false),
    };
    boundary::settle_with(
        tx,
        result,
        phase,
        explicit,
        commit,
        Error::Database,
        |message| Error::Invalid(message.into()),
    )
}
