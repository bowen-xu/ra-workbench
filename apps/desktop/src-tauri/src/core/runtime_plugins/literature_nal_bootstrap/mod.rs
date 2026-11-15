//! Fixed Core I/O for the original one-transaction NAL startup chain.
mod capture;
pub(crate) mod commands;
mod error;
mod execute;
pub(crate) mod merge_import;
mod metadata;
mod outcome;
mod prepare;
mod probes;
mod readset;
mod resources;
mod rows;
mod state_load;

pub(crate) use capture::PreparedBootstrapSource;
pub(crate) use error::CaptureError;
pub(crate) use execute::{apply, execute_in, load_link_annotations_in, load_mutation_state_in};
pub(crate) use outcome::{
    BootstrapCause, BootstrapFailure, BootstrapOutcome, BootstrapProblem, FailureContext,
};
pub(crate) use prepare::{capture_facts, capture_source};
pub(crate) use probes::PreparedBootstrapFacts;
pub(crate) use resources::BootstrapResources;
pub(crate) use state_load::{
    CompleteNalLoadError, DecodedNalState, NalLoadResources, load_complete,
};

#[cfg(test)]
mod tests;
