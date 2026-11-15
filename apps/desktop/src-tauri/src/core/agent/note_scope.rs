//! Typed saved-note scope shared by queue and same-turn commands.
use super::note_submission::source::Resource;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Scope {
    Note { resource_id: Uuid },
    ReadingNote { resource_id: Uuid },
}
impl From<Resource> for Scope {
    fn from(resource: Resource) -> Self {
        match resource {
            Resource::Page(resource_id) => Self::Note { resource_id },
            Resource::Reading(resource_id) => Self::ReadingNote { resource_id },
        }
    }
}
impl Scope {
    pub(super) fn resource(self) -> Resource {
        match self {
            Self::Note { resource_id } => Resource::Page(resource_id),
            Self::ReadingNote { resource_id } => Resource::Reading(resource_id),
        }
    }
}
