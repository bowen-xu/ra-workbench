//! Original finite reading-history queries and one autocommit archive write.
mod capture;
pub(crate) mod failure;
pub(crate) mod resources;
pub(crate) use capture::{archive, capture, snapshot_attachments};
