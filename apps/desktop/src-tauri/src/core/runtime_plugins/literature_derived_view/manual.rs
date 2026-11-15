//! Original manual view operations and confirmed suspension on the original transaction.
use super::*;
use crate::{
    ai::structured_resource_validation::Flow,
    hbs_storage::boundary::{self, BoundaryError},
};
use research_workbench_native_sdk::derived_view::manual::DerivedViewInputV1;

pub(crate) trait DerivedPolicy {
    fn validate(&mut self, input: DerivedValidationInputV1) -> Result<(), DerivedError>;
    fn provenance(&mut self, current: String, review: String) -> Result<String, DerivedError>;
}
#[derive(Clone)]
pub(crate) enum DerivedRequest {
    List {
        project_id: String,
    },
    Save {
        draft: Box<DerivedResearchViewDraft>,
    },
}
#[derive(Debug)]
pub(crate) enum DerivedResult {
    Views(Vec<DerivedResearchView>),
    View(Box<DerivedResearchView>),
}
pub(crate) fn execute(
    sql: &DerivedSql,
    db: &mut Connection,
    policy: &mut dyn DerivedPolicy,
    request: DerivedRequest,
) -> Result<DerivedResult, DerivedError> {
    match request {
        DerivedRequest::List { project_id } => {
            list_derived_views_from(sql, db, &project_id).map(DerivedResult::Views)
        }
        DerivedRequest::Save { draft } => save_derived_view_from(sql, policy, true, db, *draft)
            .map(Box::new)
            .map(DerivedResult::View),
    }
}
pub(super) fn settle<T>(
    db: &Connection,
    tx: Transaction<'_>,
    result: Result<T, DerivedError>,
    explicit: bool,
) -> Result<T, DerivedError> {
    let result: Result<T, Flow<DerivedError, Box<DerivedViewInputV1>>> =
        result.map_err(|cause| match cause {
            DerivedError::Need(input) => Flow::Need(input),
            error => Flow::original(error),
        });
    let settled = boundary::settle_with(
        tx,
        result,
        "write",
        explicit,
        true,
        DerivedError::Database,
        |cause| DerivedError::InvalidInput(cause.into()),
    );
    match boundary::confirm_with(db, settled, explicit, |cause| {
        DerivedError::InvalidInput(cause.into())
    }) {
        Ok(value) => Ok(value),
        Err(BoundaryError::Pause(pause)) => Err(DerivedError::Need(pause.need())),
        Err(BoundaryError::Failure(failure)) if explicit => {
            Err(DerivedError::Settled(Box::new(failure)))
        }
        Err(BoundaryError::Failure(failure)) => Err(failure.failure.error),
    }
}
