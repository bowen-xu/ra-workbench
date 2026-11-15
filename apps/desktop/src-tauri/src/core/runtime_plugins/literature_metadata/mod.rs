//! Core storage adapters for five original metadata consumers, not domain policy.
pub(crate) mod binding;
pub(crate) mod capture;
pub(crate) mod commands;
mod diagnostics;
mod execute;
mod outcome;
mod resources;

pub(crate) use execute::{MetadataExecution, apply};
pub(crate) use outcome::{MetadataFailure, MetadataPhaseOutcome, MetadataProblem};
pub(crate) use resources::MetadataResources;
