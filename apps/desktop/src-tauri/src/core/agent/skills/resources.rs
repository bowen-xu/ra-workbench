//! Demand reads of files within a registered skill package. Resources carry their
//! own byte revision; catalog revisions freeze SKILL.md and policy, not the tree.
use super::files::{SkillReadError, open_skill_directory, read_from_directory};
use crate::agent::instructions::{InstructionReadError, InstructionRootIdentity};
use research_agent_core::skills::{
    SkillDescriptor, SkillError, SkillIdentity, accept_loaded_skill,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRequest {
    pub identity: SkillIdentity,
    /// UTF-8 slash-separated path relative to the directory containing SKILL.md.
    pub path: String,
    pub expected_revision: Option<String>,
}
impl ResourceRequest {
    pub fn validate(&self) -> Result<(), SkillError> {
        if self.path.is_empty()
            || self.path.len() > 1024
            || self
                .path
                .chars()
                .any(|ch| ch.is_control() || ch == '\\' || ch == ':')
            || self.path.split('/').count() > 32
            || self
                .path
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
            || self.expected_revision.as_ref().is_some_and(|revision| {
                revision.len() != 64
                    || !revision
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        {
            return Err(SkillError::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct LoadedResource {
    pub identity: SkillIdentity,
    pub path: String,
    pub revision: String,
    pub text: String,
}

pub fn load_resource(
    root: &Path,
    selected: &SkillDescriptor,
    explicit: bool,
    request: &ResourceRequest,
    max_bytes: usize,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<LoadedResource, SkillReadError> {
    selected.validate()?;
    request.validate()?;
    if selected.identity != request.identity {
        return Err(SkillError::Stale.into());
    }
    if !explicit && !selected.allow_implicit_invocation {
        return Err(SkillError::ExplicitRequired.into());
    }
    if max_bytes == 0 || max_bytes > 1024 * 1024 {
        return Err(SkillError::Budget.into());
    }
    let source = Path::new(&selected.identity.source_id);
    let package = open_skill_directory(root, source, &selected.identity.environment_id, authorize)?;
    let check_version = || -> Result<(), SkillReadError> {
        let mut parsed = read_from_directory(&package, source, &selected.identity.environment_id)?;
        if selected.identity.environment_id == "application"
            && let Some(text) =
                crate::system_documents::skill_override(root, &selected.identity.source_id)
                    .map_err(|_| InstructionReadError::Unavailable)?
        {
            parsed = super::parse_document(
                "application",
                &selected.identity.source_id,
                &selected.name,
                text.as_bytes(),
            )?;
        }
        accept_loaded_skill(selected, parsed.loaded, explicit, 1024 * 1024)?;
        Ok(())
    };
    check_version()?;
    // Reopen descendants from the pinned package, never resolve the source path
    // again. Root/package replacement cannot redirect the resource lookup.
    let mut directory = package
        .optional_child(Path::new("."))?
        .ok_or(SkillReadError::NotFound)?;
    let parts: Vec<_> = request.path.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        directory = directory
            .optional_child(Path::new(part))?
            .ok_or(SkillReadError::NotFound)?;
    }
    let file = directory
        .config_file(parts[parts.len() - 1])?
        .ok_or(SkillReadError::NotFound)?;
    if file
        .metadata()
        .map_err(|_| InstructionReadError::Unavailable)?
        .len()
        > max_bytes as u64
    {
        return Err(SkillError::Budget.into());
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| InstructionReadError::Unavailable)?;
    if bytes.len() > max_bytes {
        return Err(SkillError::Budget.into());
    }
    if selected.identity.environment_id == "application" {
        let relative = source
            .parent()
            .ok_or(InstructionReadError::InvalidScope)?
            .join(&request.path);
        if let Some(text) = crate::system_documents::skill_override(
            root,
            relative
                .to_str()
                .ok_or(InstructionReadError::InvalidScope)?,
        )
        .map_err(|_| InstructionReadError::Unavailable)?
        {
            bytes = text.into_bytes();
        }
    }
    if bytes.len() > max_bytes {
        return Err(SkillError::Budget.into());
    }
    let revision = format!("{:x}", Sha256::digest(&bytes));
    if request
        .expected_revision
        .as_ref()
        .is_some_and(|expected| expected != &revision)
    {
        return Err(SkillError::Stale.into());
    }
    let text = String::from_utf8(bytes).map_err(|_| super::SkillParseError::Encoding)?;
    if text.contains('\0') {
        return Err(super::SkillParseError::Invalid.into());
    }
    check_version()?;
    Ok(LoadedResource {
        identity: selected.identity.clone(),
        path: request.path.clone(),
        revision,
        text,
    })
}

#[cfg(all(test, unix))]
#[path = "resources/tests.rs"]
mod tests;
