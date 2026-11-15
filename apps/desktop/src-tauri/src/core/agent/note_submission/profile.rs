//! Note host identity and error adaptation for shared profile snapshots.
use super::{Error, Resource};
use crate::agent::config::{profile_snapshot, profiles::ProfileKind};
use research_agent_core::{
    config::{InstructionConfigPatch, InstructionConfigSnapshot},
    instructions::InstructionFragment,
    submission::SubmissionInput,
};
use std::path::Path;

fn identity(resource: Resource) -> (ProfileKind, String) {
    match resource {
        Resource::Page(id) => (ProfileKind::Note, format!("note:{id}")),
        Resource::Reading(id) => (ProfileKind::ReadingNote, format!("reading-note:{id}")),
    }
}

pub(super) fn freeze(
    root: &Path,
    resource: Resource,
    overrides: [Option<InstructionConfigPatch>; 2],
) -> Result<(Vec<InstructionFragment>, Vec<InstructionConfigSnapshot>), Error> {
    let (kind, scope) = identity(resource);
    profile_snapshot::freeze(root, kind, scope, overrides).map_err(|_| Error::ProfileUnavailable)
}

pub(super) fn validate(input: &SubmissionInput, resource: Resource) -> Result<(), Error> {
    crate::agent::skills::application::validate(input).map_err(|_| Error::InvalidInput)?;
    let (kind, scope) = identity(resource);
    profile_snapshot::validate(input, kind, &scope).map_err(|_| Error::InvalidInput)
}
