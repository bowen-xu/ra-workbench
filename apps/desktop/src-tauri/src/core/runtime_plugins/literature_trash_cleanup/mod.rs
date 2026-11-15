//! Original expiry transactions over fixed package resources and a checked plan.
mod execute;
mod outcome;
mod plan;
mod resources;

pub(crate) use execute::purge;
pub(crate) use outcome::{TrashCleanupFailure, TrashCleanupOutcome, TrashCleanupProblem};
pub(crate) use plan::{CheckedCleanupPlan, check_preparation};
pub(crate) use resources::TrashCleanupResources;
