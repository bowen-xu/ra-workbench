//! Reference records retain Core SQL/log, identity and transaction ownership.
pub(crate) mod acceptance;
pub(crate) mod commands;
mod data;
mod execute;
mod outcome;
mod resources;

pub(crate) use data::list;
pub(crate) use execute::save;
pub(crate) use outcome::{ReferenceFailure, ReferenceOutcome, ReferenceProblem, ReferenceProgress};
pub(crate) use resources::ReferenceResources;

use research_workbench_native_sdk::reference_records::{ReferenceRecord, ReferenceRecordDraft};

pub(crate) enum ReferenceRequest {
    List { paper_id: String },
    Save { draft: Box<ReferenceRecordDraft> },
}

#[derive(Debug)]
pub(crate) enum ReferenceResult {
    Records(Vec<ReferenceRecord>),
    Record(Box<ReferenceRecord>),
}

#[cfg(test)]
mod tests;
