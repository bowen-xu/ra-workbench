//! Application-owned skill catalog IPC.
//!
//! Adapted from Codex `skills/list`: native code chooses the configured root,
//! returns bounded metadata plus per-skill failures, and never exposes skill
//! bodies or accepts a renderer-supplied filesystem path.

use super::{instructions::InstructionReadError, skills::files::SkillReadError};
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogResponse {
    pub schema_version: u8,
    pub environment_id: &'static str,
    pub skills: Vec<SkillSummary>,
    pub errors: Vec<SkillCatalogError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub source_id: String,
    pub name: String,
    pub description: String,
    pub revision: String,
    pub allow_implicit_invocation: bool,
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogError {
    pub source_id: String,
    pub code: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    Unavailable,
    InvalidCatalog,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetSkillEnabledRequest {
    pub source_id: String,
    pub revision: String,
    pub enabled: bool,
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_list_application_skills(
    store: State<'_, AppStore>,
) -> Result<SkillCatalogResponse, Error> {
    let root = store.agent_configuration_directory().to_path_buf();
    tokio::task::spawn_blocking(move || list_from_root(&root))
        .await
        .map_err(|_| Error::Unavailable)?
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_set_application_skill_enabled(
    store: State<'_, AppStore>,
    request: SetSkillEnabledRequest,
) -> Result<SkillCatalogResponse, Error> {
    let root = store.agent_configuration_directory().to_path_buf();
    tokio::task::spawn_blocking(move || set_from_root(&root, request))
        .await
        .map_err(|_| Error::Unavailable)?
}

pub(crate) fn set_from_root(
    root: &Path,
    request: SetSkillEnabledRequest,
) -> Result<SkillCatalogResponse, Error> {
    let current = list_from_root(root)?;
    let matches = current
        .skills
        .iter()
        .any(|skill| skill.source_id == request.source_id && skill.revision == request.revision);
    if !matches {
        return Err(Error::InvalidCatalog);
    }
    super::config::skills::set_enabled(root, &request.source_id, request.enabled)
        .map_err(map_config_error)?;
    list_from_root(root)
}

pub(crate) fn list_from_root(root: &Path) -> Result<SkillCatalogResponse, Error> {
    let discovered = super::skills::application::discover(root).map_err(map_root_error)?;
    let rules = super::config::skills::read(root).map_err(map_config_error)?;
    Ok(SkillCatalogResponse {
        schema_version: 1,
        environment_id: "application",
        skills: discovered
            .skills
            .into_iter()
            .map(|skill| {
                let source_id = skill.identity.source_id;
                SkillSummary {
                    source_id: source_id.clone(),
                    name: skill.name,
                    description: skill.description,
                    revision: skill.identity.revision,
                    allow_implicit_invocation: skill.allow_implicit_invocation,
                    enabled: rules.enabled(&source_id),
                }
            })
            .collect(),
        errors: discovered
            .rejected
            .into_iter()
            .map(|(source_id, error)| SkillCatalogError {
                source_id,
                code: error_code(&error),
            })
            .collect(),
    })
}

fn map_config_error(error: super::config::ConfigReadError) -> Error {
    match error {
        super::config::ConfigReadError::Read(error) => map_root_error(error),
        super::config::ConfigReadError::Invalid => Error::InvalidCatalog,
    }
}

fn map_root_error(error: InstructionReadError) -> Error {
    match error {
        InstructionReadError::Unavailable | InstructionReadError::UnsupportedPlatform => {
            Error::Unavailable
        }
        InstructionReadError::InvalidScope
        | InstructionReadError::Unauthorized
        | InstructionReadError::Budget
        | InstructionReadError::Encoding => Error::InvalidCatalog,
    }
}

fn error_code(error: &SkillReadError) -> &'static str {
    match error {
        SkillReadError::NotFound => "not_found",
        SkillReadError::Parse(_) => "invalid_document",
        SkillReadError::Selection(_) => "invalid_selection",
        SkillReadError::Read(InstructionReadError::Budget) => "budget_exceeded",
        SkillReadError::Read(InstructionReadError::Encoding) => "invalid_encoding",
        SkillReadError::Read(InstructionReadError::Unavailable) => "unavailable",
        SkillReadError::Read(InstructionReadError::UnsupportedPlatform) => "unsupported_platform",
        SkillReadError::Read(
            InstructionReadError::InvalidScope | InstructionReadError::Unauthorized,
        ) => "unauthorized",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_bounded_metadata_and_reports_malformed_skills_without_bodies() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let valid = root.join(".agents/skills/review");
        let invalid = root.join(".agents/skills/broken");
        fs::create_dir_all(&valid).unwrap();
        fs::create_dir_all(&invalid).unwrap();
        fs::write(
            valid.join("SKILL.md"),
            "---\ndescription: Review prose\n---\nprivate body",
        )
        .unwrap();
        fs::write(invalid.join("SKILL.md"), "invalid").unwrap();

        let response = list_from_root(&root).unwrap();
        assert_eq!(response.schema_version, 1);
        assert_eq!(response.environment_id, "application");
        assert_eq!(response.skills.len(), 1);
        assert_eq!(response.skills[0].name, "review");
        assert!(response.skills[0].enabled);
        assert_eq!(
            response.skills[0].source_id,
            ".agents/skills/review/SKILL.md"
        );
        assert_eq!(response.errors.len(), 1);
        assert_eq!(
            response.errors[0].source_id,
            ".agents/skills/broken/SKILL.md"
        );
        assert_eq!(response.errors[0].code, "invalid_document");
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("private body"));
        assert!(!json.contains(root.to_string_lossy().as_ref()));
    }

    #[test]
    fn missing_configuration_root_is_an_empty_catalog() {
        let root = std::path::PathBuf::from("/definitely/missing/research-assistant-skill-root");
        let response = list_from_root(&root).unwrap();
        assert!(response.skills.is_empty());
        assert!(response.errors.is_empty());
    }

    #[test]
    fn set_requires_current_revision_and_changes_freezing_state() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let skill = root.join(".agents/skills/review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\ndescription: Review prose\n---\nbody",
        )
        .unwrap();
        let listed = list_from_root(&root).unwrap();
        let revision = listed.skills[0].revision.clone();
        assert!(
            super::super::skills::application::freeze(&root, "$review")
                .unwrap()
                .len()
                == 1
        );
        assert!(matches!(
            set_from_root(
                &root,
                SetSkillEnabledRequest {
                    source_id: ".agents/skills/review/SKILL.md".into(),
                    revision: "stale".into(),
                    enabled: false,
                }
            ),
            Err(Error::InvalidCatalog)
        ));
        let disabled = set_from_root(
            &root,
            SetSkillEnabledRequest {
                source_id: ".agents/skills/review/SKILL.md".into(),
                revision: revision.clone(),
                enabled: false,
            },
        )
        .unwrap();
        assert_eq!(disabled.skills[0].revision, revision);
        assert!(!disabled.skills[0].enabled);
        assert!(
            super::super::skills::application::freeze(&root, "$review")
                .unwrap()
                .is_empty()
        );
    }
}
