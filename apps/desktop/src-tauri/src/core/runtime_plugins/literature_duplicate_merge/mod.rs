//! Fixed transaction owner for the independently hosted literature merge.
pub(crate) mod commands;
mod execute;
mod load;
mod outcome;
pub(crate) mod resources;

pub(crate) use execute::{apply, verify_preparation};
pub(crate) use load::{CapturedMerge, capture};
pub(crate) use outcome::{MergeCause, MergeFailure, MergeOutcome, MergePhase, MergeProblem};
pub(crate) use resources::MergeResources;

#[cfg(test)]
mod tests;
