//! Fixed Core I/O for the original atomic NAL annotation and shared Note creation.
pub(crate) mod commands;
mod execute;
mod outcome;
mod resources;

pub(crate) use execute::{apply, apply_link_in};
pub(crate) use outcome::{
    AnnotationCause, AnnotationFailure, AnnotationOutcome, AnnotationPhase, AnnotationProblem,
    AnnotationRejection,
};
pub(crate) use resources::AnnotationResources;

#[cfg(test)]
mod tests;
