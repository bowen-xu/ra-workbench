//! Original workflow storage and shared task ownership. Native supplies domain decisions.
pub(crate) mod attempt_resources;
pub(crate) mod attempt_store;
pub(crate) mod capture;
pub(crate) mod commands;
pub(crate) mod conversation_store;
pub(crate) mod creation_policy;
pub(crate) mod creation_resources;
pub(crate) mod creation_store;
pub(crate) mod docling_resources;
pub(crate) mod error;
mod execution;
pub(crate) mod failure;
pub(crate) mod failure_store;
pub(crate) mod manager;
pub(crate) mod recovery_resources;
pub(crate) mod resources;
pub(crate) mod workflow_failure;
pub(crate) mod workflow_resources;
