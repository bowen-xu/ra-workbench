//! Mutable application instruction documents. Content is active on save; D/B is
//! only a review projection. Agent writes require one exact host approval.
#[path = "approvals.rs"]
pub mod approvals;
#[path = "backend.rs"]
pub mod backend;
#[path = "commands.rs"]
pub mod commands;
#[path = "registry.rs"]
mod registry;
#[path = "storage.rs"]
mod storage;
pub(crate) use registry::id_from_path as document_id_from_path;
pub use storage::{Document, prompt, read, skill_override, write};
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
