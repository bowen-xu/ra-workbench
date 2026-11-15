//! Three fixed duplicate-review ports. Domain policy stays in the native host.
pub(crate) mod commands;
mod execute;
mod resources;

pub(crate) use execute::{DecisionSqlState, execute_decision};
use research_workbench_native_sdk::duplicate_review::{
    DuplicateCandidate, DuplicateDecisionRequestV1, MergeOperation,
};
pub(crate) use resources::DuplicateResources;

#[derive(Debug)]
pub(crate) enum ReviewRequest {
    Candidates,
    History,
    Decide(DuplicateDecisionRequestV1),
}

#[derive(Debug)]
pub(crate) enum ReviewResult {
    Candidates(Vec<DuplicateCandidate>),
    History(Vec<MergeOperation>),
    Decided,
}

#[cfg(test)]
pub(crate) mod tests;
