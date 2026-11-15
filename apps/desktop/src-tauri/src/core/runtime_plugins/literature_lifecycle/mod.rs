//! Mechanical execution of six existing lifecycle operations over fixed grants.
pub(crate) mod commands;
mod execute;
mod outcome;
mod resources;

pub(crate) use execute::{apply, check_preparation};
pub(crate) use outcome::{LifecycleFailure, LifecycleOutcome, LifecycleProblem};
pub(crate) use resources::LifecycleResources;

#[cfg(test)]
mod tests;
