//! Original index task and vector execution over a fixed application port.
pub(crate) mod host;
pub(crate) mod manager;
pub(crate) mod model;
pub(crate) mod reconciler;
pub(crate) mod worker;

mod decisions;
mod execute;
mod failure;
mod reconciliation_repository;
mod repository;
mod resources;
mod runtime_host;
pub(crate) use execute::{Operation as IndexOperation, Output as IndexOutput};
pub(crate) use failure::Error as IndexError;
pub(crate) use resources::IndexResources;
pub(crate) use runtime_host::RuntimeIndexHost;

mod generation_repository;

pub(crate) mod source;

pub(crate) mod commands;

/// Settings and backfill retain the original borrowed connection and partial
/// write order. This family performs no Native wait or additional transaction.
pub(crate) fn backfill_in(
    db: &rusqlite::Connection,
    resources: &IndexResources,
) -> Result<i64, IndexError> {
    let cache = decisions::Cache::default();
    let run = decisions::IndexRun::new(resources, &cache);
    generation_repository::queue_current_concept_backfill(&run, db)
}
