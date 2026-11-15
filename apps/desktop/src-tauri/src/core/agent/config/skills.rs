//! Application skill enablement persisted in the app-owned Agent config.
//!
//! The edit shape and comment-preserving mutation follow Codex
//! `ConfigEdit::SetSkillConfig`; source selectors are application-relative IDs
//! instead of absolute Codex workspace paths.

use super::{ConfigDocument, ConfigReadError, MAX_CONFIG_BYTES};
use crate::agent::instructions::InstructionReadError;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
    path::{Component, Path},
};
use tempfile::NamedTempFile;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

const CONFIG_PATH: &str = "agent/config.toml";

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SkillsConfig {
    #[serde(default)]
    config: Vec<SkillConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillConfig {
    source_id: String,
    enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SkillRules(BTreeMap<String, bool>);

impl SkillsConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigReadError> {
        let mut sources = HashSet::new();
        if self.config.len() > 256
            || self.config.iter().any(|entry| {
                !valid_source_id(&entry.source_id) || !sources.insert(entry.source_id.as_str())
            })
        {
            return Err(ConfigReadError::Invalid);
        }
        Ok(())
    }

    fn into_rules(self) -> SkillRules {
        SkillRules(
            self.config
                .into_iter()
                .map(|entry| (entry.source_id, entry.enabled))
                .collect(),
        )
    }
}

impl SkillRules {
    pub(crate) fn enabled(&self, source_id: &str) -> bool {
        self.0.get(source_id).copied().unwrap_or(true)
    }
}

pub(crate) fn read(root: &Path) -> Result<SkillRules, ConfigReadError> {
    Ok(super::read_user_document(root)?
        .map(|(document, _)| document.skills.into_rules())
        .unwrap_or_default())
}

pub(crate) fn set_enabled(
    root: &Path,
    source_id: &str,
    enabled: bool,
) -> Result<(), ConfigReadError> {
    if !valid_source_id(source_id) {
        return Err(ConfigReadError::Invalid);
    }
    let root = validate_root(root)?;
    let config_path = root.join(CONFIG_PATH);
    let serialized = match fs::read_to_string(&config_path) {
        Ok(value) if value.len() <= MAX_CONFIG_BYTES => value,
        Ok(_) => return Err(InstructionReadError::Budget.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(InstructionReadError::Unavailable.into()),
    };
    let mut document = if serialized.is_empty() {
        let mut document = DocumentMut::new();
        document["schema_version"] = value(1);
        document
    } else {
        serialized
            .parse::<DocumentMut>()
            .map_err(|_| ConfigReadError::Invalid)?
    };
    if !apply(&mut document, source_id, enabled)? {
        return Ok(());
    }
    let output = document.to_string();
    if output.len() > MAX_CONFIG_BYTES {
        return Err(InstructionReadError::Budget.into());
    }
    let parsed: ConfigDocument = toml::from_str(&output).map_err(|_| ConfigReadError::Invalid)?;
    if parsed.schema_version != 1 {
        return Err(ConfigReadError::Invalid);
    }
    parsed.skills.validate()?;
    parsed.profiles.validate()?;
    write_atomically(&config_path, output.as_bytes())
}

fn apply(
    document: &mut DocumentMut,
    source_id: &str,
    enabled: bool,
) -> Result<bool, ConfigReadError> {
    let root = document.as_table_mut();
    if !root.contains_key("skills") {
        if enabled {
            return Ok(false);
        }
        root.insert("skills", Item::Table(Table::new()));
    }
    let skills = root
        .get_mut("skills")
        .and_then(Item::as_table_mut)
        .ok_or(ConfigReadError::Invalid)?;
    if !skills.contains_key("config") {
        if enabled {
            return Ok(false);
        }
        skills.insert("config", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let entries = skills
        .get_mut("config")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or(ConfigReadError::Invalid)?;
    let existing = entries
        .iter()
        .position(|table| table.get("source_id").and_then(Item::as_str) == Some(source_id));
    if enabled {
        let Some(index) = existing else {
            return Ok(false);
        };
        entries.remove(index);
        if entries.is_empty() {
            skills.remove("config");
            if skills.is_empty() {
                root.remove("skills");
            }
        }
        return Ok(true);
    }
    if let Some(index) = existing {
        let entry = entries.get_mut(index).ok_or(ConfigReadError::Invalid)?;
        if entry.get("enabled").and_then(Item::as_bool) == Some(false) {
            return Ok(false);
        }
        entry["enabled"] = value(false);
    } else {
        let mut entry = Table::new();
        entry["source_id"] = value(source_id);
        entry["enabled"] = value(false);
        entries.push(entry);
    }
    Ok(true)
}

fn valid_source_id(value: &str) -> bool {
    let path = Path::new(value);
    value.len() <= 1024
        && !value.chars().any(char::is_control)
        && path.starts_with(".agents/skills")
        && path.file_name().is_some_and(|name| name == "SKILL.md")
        && path.components().count() >= 4
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn validate_root(root: &Path) -> Result<std::path::PathBuf, ConfigReadError> {
    if !root.is_absolute() {
        return Err(InstructionReadError::InvalidScope.into());
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| InstructionReadError::Unavailable)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(InstructionReadError::InvalidScope.into());
    }
    root.canonicalize()
        .map_err(|_| InstructionReadError::Unavailable.into())
}

pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), ConfigReadError> {
    let parent = path.parent().ok_or(ConfigReadError::Invalid)?;
    fs::create_dir_all(parent).map_err(|_| InstructionReadError::Unavailable)?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| InstructionReadError::Unavailable)?;
    if canonical_parent != parent {
        return Err(InstructionReadError::InvalidScope.into());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(InstructionReadError::InvalidScope.into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(InstructionReadError::Unavailable.into()),
    }
    let mut temporary =
        NamedTempFile::new_in(parent).map_err(|_| InstructionReadError::Unavailable)?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| InstructionReadError::Unavailable)?;
    temporary
        .persist(path)
        .map_err(|_| InstructionReadError::Unavailable)?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| InstructionReadError::Unavailable.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disables_and_reenables_one_skill_without_rewriting_other_config() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("agent")).unwrap();
        fs::write(
            root.join(CONFIG_PATH),
            "# keep\nschema_version = 1\n[instructions]\nmax_bytes = 2048\n",
        )
        .unwrap();
        let source = ".agents/skills/review/SKILL.md";
        set_enabled(&root, source, false).unwrap();
        assert!(!read(&root).unwrap().enabled(source));
        assert!(
            fs::read_to_string(root.join(CONFIG_PATH))
                .unwrap()
                .contains("# keep")
        );
        set_enabled(&root, source, true).unwrap();
        assert!(read(&root).unwrap().enabled(source));
        assert!(
            fs::read_to_string(root.join(CONFIG_PATH))
                .unwrap()
                .contains("max_bytes = 2048")
        );
    }

    #[test]
    fn rejects_unknown_fields_duplicates_and_unsafe_sources() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("agent")).unwrap();
        fs::write(root.join(CONFIG_PATH), "schema_version=1\n[[skills.config]]\nsource_id='.agents/skills/review/SKILL.md'\nenabled=false\nunknown=true").unwrap();
        assert!(read(&root).is_err());
        assert!(set_enabled(&root, "../outside/SKILL.md", false).is_err());
    }
}
