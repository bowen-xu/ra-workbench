//! The original deferred save transaction over reviewed package resources.
//! Domain preparation, DOI policy and result projection remain outside Core.
mod error;
mod execute;
mod resources;
mod steps;

pub(crate) use error::{SafeDatabaseError, SaveCause, SaveFailure, SaveOutcome};
pub(crate) use execute::{persist, probe_begin};
pub(crate) use resources::SaveResources;

use research_workbench_native_sdk::catalog::CatalogItemRows;

#[derive(Debug)]
pub(crate) struct CommittedCatalogRows {
    pub(crate) item_id: String,
    pub(crate) items: Vec<CatalogItemRows>,
}

#[cfg(test)]
mod tests;

pub(crate) use steps::{refresh_search, save as save_in_transaction};
