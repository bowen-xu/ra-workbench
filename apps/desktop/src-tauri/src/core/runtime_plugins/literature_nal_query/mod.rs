//! Original query-collection persistence; semantic validation remains in Native.
pub(crate) mod commands;
mod error;
mod resources;
mod save;

pub(crate) use error::{QueryCause, QueryFailure, QueryOutcome};
pub(crate) use resources::NalQueryResources;
pub(crate) use save::save;

#[cfg(test)]
mod tests;
