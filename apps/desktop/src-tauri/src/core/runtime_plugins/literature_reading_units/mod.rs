//! Fixed manual reading-unit adapter; original SQL/log and Note owners remain Core.
pub(crate) mod accepted;
pub(crate) mod commands;
pub(crate) mod confirmation;
mod data;
mod execute;
mod failure;
mod resources;
pub(crate) use execute::{apply, check_preparation};
pub(crate) use failure::{ReadingUnitFailure, ReadingUnitOutcome, ReadingUnitProblem};
pub(crate) use research_workbench_native_sdk::reading_units::ReadingUnitRequestV1 as ReadingUnitRequest;
use research_workbench_native_sdk::reading_units::StructuredReadingUnit;
pub(crate) use resources::ReadingUnitResources;
pub(crate) enum ReadingUnitResult {
    Units(Vec<StructuredReadingUnit>),
    Unit(Box<StructuredReadingUnit>),
}
#[cfg(test)]
mod tests;
