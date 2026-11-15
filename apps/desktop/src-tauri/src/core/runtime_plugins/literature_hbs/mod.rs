//! Core-private adapters for the original structured HBS validation boundary.
mod failure;
mod native_message;
mod operation;
mod policy;
mod resources;
pub(crate) use failure::{HbsOperationError, NativeFailure};
pub(crate) use operation::{
    OperationError, OperationFailure, ResourceOperationError, StructuredOperation,
    StructuredOutput, map_boundary, operation_error,
};
pub(crate) use policy::RuntimeHbsSources;
pub(crate) use resources::HbsReadingEvidenceResources;

pub(crate) enum HbsOperation {
    ReadPreservation,
    SavePreservation(serde_json::Value),
    Commit(crate::hbs_storage::Commit),
}
