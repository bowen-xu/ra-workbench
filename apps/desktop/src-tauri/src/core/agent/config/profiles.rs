//! Non-file hosts select named user instructions, never an execution policy.
use super::{ConfigReadError, read_user_document};
use research_agent_core::config::{DEFAULT_INSTRUCTION_BYTES, MAX_INSTRUCTION_BYTES};
use serde::Deserialize;
use std::path::Path;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Profiles {
    reading: Option<UserProfile>,
    note: Option<UserProfile>,
    reading_note: Option<UserProfile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserProfile {
    pub text: String,
    #[serde(default = "default_budget")]
    pub max_bytes: usize,
}
fn default_budget() -> usize {
    DEFAULT_INSTRUCTION_BYTES
}

#[derive(Clone, Copy)]
pub(crate) enum ProfileKind {
    Reading,
    Note,
    ReadingNote,
}
impl ProfileKind {
    pub fn source_id(self) -> &'static str {
        match self {
            Self::Reading => "agent/config.toml#profiles.reading",
            Self::Note => "agent/config.toml#profiles.note",
            Self::ReadingNote => "agent/config.toml#profiles.reading_note",
        }
    }
}
impl Profiles {
    pub(super) fn is_empty(&self) -> bool {
        self.reading.is_none() && self.note.is_none() && self.reading_note.is_none()
    }
    pub(super) fn validate(&self) -> Result<(), ConfigReadError> {
        for profile in [&self.reading, &self.note, &self.reading_note]
            .into_iter()
            .flatten()
        {
            if profile.max_bytes > MAX_INSTRUCTION_BYTES || profile.text.contains('\0') {
                return Err(ConfigReadError::Invalid);
            }
        }
        Ok(())
    }
}

/// Root comes from AppStore. No IPC paths, inheritance from documents, or reads
/// of linked instruction files. The whole config byte hash versions the choice.
pub(crate) fn read_user_profile(
    root: &Path,
    kind: ProfileKind,
) -> Result<Option<(UserProfile, String)>, ConfigReadError> {
    let Some((document, revision)) = read_user_document(root)? else {
        return Ok(None);
    };
    let profile = match kind {
        ProfileKind::Reading => document.profiles.reading,
        ProfileKind::Note => document.profiles.note,
        ProfileKind::ReadingNote => document.profiles.reading_note,
    };
    Ok(profile.map(|profile| (profile, revision)))
}
