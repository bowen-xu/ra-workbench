//! App-owned TOML layers. No Codex home, credentials or model-facing paths.
use super::instructions::{InstructionReadError, InstructionRootIdentity, directory::Directory};
use research_agent_core::config::{
    ConfigLayer, ConfigOrigin, ConfigTier, InstructionConfigPatch, resolve_instruction_config,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Component, Path},
};

const MAX_CONFIG_BYTES: usize = 64 * 1024;
#[path = "config/profile_snapshot.rs"]
pub(crate) mod profile_snapshot;
#[path = "config/profiles.rs"]
pub(crate) mod profiles;
#[path = "config/skills.rs"]
pub(crate) mod skills;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigOverride {
    #[serde(default)]
    pub max_bytes: Option<usize>,
    #[serde(default)]
    pub fallback_filenames: Option<Vec<String>>,
}

impl From<ConfigOverride> for research_agent_core::config::InstructionConfigPatch {
    fn from(value: ConfigOverride) -> Self {
        Self {
            max_bytes: value.max_bytes,
            fallback_filenames: value.fallback_filenames,
        }
    }
}

/// Only call with the application configuration directory obtained by the host
/// (or a private fixture root). Missing config roots inherit without creating files.
pub(crate) fn read_user_layer(root: &Path) -> Result<Option<ConfigLayer>, ConfigReadError> {
    Ok(
        read_user_document(root)?.map(|(document, revision)| ConfigLayer {
            origin: ConfigOrigin {
                tier: ConfigTier::User,
                source_id: "agent/config.toml".into(),
                revision,
            },
            instructions: document.instructions,
        }),
    )
}

/// Read the application-owned Codex feature choice. Missing configuration uses
/// the upstream default; callers must freeze this value with the submission.
pub fn remote_compaction_v2(root: &Path) -> Result<bool, ConfigReadError> {
    Ok(read_user_document(root)?
        .and_then(|(document, _)| document.features.remote_compaction_v2)
        .unwrap_or(true))
}

pub(crate) fn freeze_remote_compaction(
    root: &Path,
    model: &research_agent_core::protocol::ModelIdentity,
) -> Result<Option<bool>, ConfigReadError> {
    if model.provider_id == "openai" {
        remote_compaction_v2(root).map(Some)
    } else {
        Ok(None)
    }
}

fn read_user_document(root: &Path) -> Result<Option<(ConfigDocument, String)>, ConfigReadError> {
    if !root.is_absolute() {
        return Err(InstructionReadError::InvalidScope.into());
    }
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(InstructionReadError::Unavailable.into()),
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err(InstructionReadError::InvalidScope.into());
        }
        Ok(_) => {}
    }
    let canonical = root
        .canonicalize()
        .map_err(|_| InstructionReadError::Unavailable)?;
    // Until handle-relative reads are implemented on Windows, a missing user
    // config still inherits. Never read a present file through an unsafe fallback.
    #[cfg(not(unix))]
    match std::fs::symlink_metadata(canonical.join("agent/config.toml")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(InstructionReadError::Unavailable.into()),
        Ok(_) => return Err(InstructionReadError::UnsupportedPlatform.into()),
    }
    #[cfg(unix)]
    read_document(&canonical, Path::new("agent/config.toml"), |_| Ok(()))
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigReadError {
    #[error("configuration source cannot be read")]
    Read(#[from] InstructionReadError),
    #[error("configuration schema or setting is invalid")]
    Invalid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigDocument {
    schema_version: u32,
    #[serde(default)]
    instructions: InstructionConfigPatch,
    #[serde(default)]
    profiles: profiles::Profiles,
    #[serde(default)]
    skills: skills::SkillsConfig,
    #[serde(default)]
    features: Features,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Features {
    #[serde(default)]
    remote_compaction_v2: Option<bool>,
}

/// Root is selected by the native host. Authorization runs on its opened identity
/// BEFORE probing the relative config path. A missing layer inherits; malformed
/// files and symlinks fail. Call on a blocking worker; no IPC path inputs.
/// The byte hash versions the exact successfully parsed source, including comments.
pub fn read_layer(
    root: &Path,
    relative: &Path,
    tier: ConfigTier,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<Option<ConfigLayer>, ConfigReadError> {
    let Some((document, revision)) = read_document(root, relative, authorize)? else {
        return Ok(None);
    };
    if tier != ConfigTier::User
        && (!document.profiles.is_empty() || document.features.remote_compaction_v2.is_some())
    {
        return Err(ConfigReadError::Invalid);
    }
    Ok(Some(ConfigLayer {
        origin: ConfigOrigin {
            tier,
            source_id: relative.to_string_lossy().into_owned(),
            revision,
        },
        instructions: document.instructions,
    }))
}

fn read_document(
    root: &Path,
    relative: &Path,
    authorize: impl FnOnce(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<Option<(ConfigDocument, String)>, ConfigReadError> {
    let components: Vec<_> = relative.components().collect();
    if !root.is_absolute()
        || root
            .canonicalize()
            .map_err(|_| InstructionReadError::Unavailable)?
            != root
        || components.is_empty()
        || components.len() > 64
        || components
            .iter()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(InstructionReadError::InvalidScope.into());
    }
    let source_id = relative.to_str().ok_or(ConfigReadError::Invalid)?;
    let mut directory = Directory::root(root)?;
    let (device, inode) = directory.identity()?;
    authorize(&InstructionRootIdentity {
        canonical_path: root,
        device,
        inode,
    })?;
    for component in &components[..components.len() - 1] {
        let Some(child) = directory.optional_child(Path::new(component.as_os_str()))? else {
            return Ok(None);
        };
        directory = child;
    }
    let name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ConfigReadError::Invalid)?;
    let Some(file) = directory.config_file(name)? else {
        return Ok(None);
    };
    if file
        .metadata()
        .map_err(|_| InstructionReadError::Unavailable)?
        .len()
        > MAX_CONFIG_BYTES as u64
    {
        return Err(InstructionReadError::Budget.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| InstructionReadError::Unavailable)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(InstructionReadError::Budget.into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| InstructionReadError::Encoding)?;
    // Do not forward parser errors: they can quote private configuration values.
    let document: ConfigDocument = toml::from_str(text).map_err(|_| ConfigReadError::Invalid)?;
    if document.schema_version != 1 {
        return Err(ConfigReadError::Invalid);
    }
    let layer = ConfigLayer {
        origin: ConfigOrigin {
            tier: ConfigTier::User,
            source_id: source_id.into(),
            revision: format!("{:x}", Sha256::digest(&bytes)),
        },
        instructions: document.instructions.clone(),
    };
    resolve_instruction_config(std::slice::from_ref(&layer))
        .map_err(|_| ConfigReadError::Invalid)?;
    document.profiles.validate()?;
    document.skills.validate()?;
    Ok(Some((document, layer.origin.revision)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn remote_compaction_feature_is_user_only_and_defaults_to_v2() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        assert!(remote_compaction_v2(&root).unwrap());
        fs::create_dir(root.join("agent")).unwrap();
        let config = root.join("agent/config.toml");
        fs::write(
            &config,
            "schema_version=1\n[features]\nremote_compaction_v2=false\n",
        )
        .unwrap();
        assert!(!remote_compaction_v2(&root).unwrap());
        assert_eq!(
            read_layer(
                &root,
                Path::new("agent/config.toml"),
                ConfigTier::Workspace,
                |_| Ok(())
            ),
            Err(ConfigReadError::Invalid)
        );
        fs::write(
            &config,
            "schema_version=1\n[features]\nremote_compaction_v2=true\n",
        )
        .unwrap();
        assert!(remote_compaction_v2(&root).unwrap());
        fs::write(
            &config,
            "schema_version=1\n[features]\nremote_compaction_v2='false'\n",
        )
        .unwrap();
        assert_eq!(remote_compaction_v2(&root), Err(ConfigReadError::Invalid));
    }

    #[test]
    fn note_profiles_are_user_only_and_do_not_change_workspace_instruction_settings() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let relative = Path::new("agent.toml");
        fs::write(root.join(relative), "schema_version=1\n[instructions]\nmax_bytes=99\n[profiles.note]\ntext='Synthetic profile'").unwrap();
        assert_eq!(
            read_layer(&root, relative, ConfigTier::Workspace, |_| Ok(())),
            Err(ConfigReadError::Invalid)
        );
        let user = read_layer(&root, relative, ConfigTier::User, |_| Ok(()))
            .unwrap()
            .unwrap();
        assert_eq!(user.instructions.max_bytes, Some(99));
    }

    #[test]
    fn user_config_root_is_optional_app_owned_and_rejects_links() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("configuration");
        assert_eq!(read_user_layer(&root).unwrap(), None);
        fs::create_dir_all(root.join("agent")).unwrap();
        fs::write(
            root.join("agent/config.toml"),
            "schema_version = 1\n[instructions]\nmax_bytes = 19",
        )
        .unwrap();
        let layer = read_user_layer(&root).unwrap().unwrap();
        assert_eq!(layer.origin.tier, ConfigTier::User);
        assert_eq!(layer.instructions.max_bytes, Some(19));
        let link = root.with_file_name("linked");
        symlink(&root, &link).unwrap();
        assert!(read_user_layer(&link).is_err());
        assert!(read_user_layer(Path::new("missing-relative-root")).is_err());
    }

    #[test]
    fn missing_layers_inherit_and_content_changes_version_the_layer() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let relative = Path::new("agent/config.toml");
        assert_eq!(
            read_layer(&root, relative, ConfigTier::User, |_| Ok(())).unwrap(),
            None
        );
        fs::create_dir(root.join("agent")).unwrap();
        let text = "schema_version = 1\n[instructions]\nmax_bytes = 2048\nfallback_filenames = ['TEAM.md']\n";
        fs::write(root.join(relative), text).unwrap();
        let first = read_layer(&root, relative, ConfigTier::User, |_| Ok(()))
            .unwrap()
            .unwrap();
        assert_eq!(first.instructions.max_bytes, Some(2048));
        assert_eq!(first.origin.source_id, "agent/config.toml");
        fs::write(root.join(relative), format!("{text}# new version\n")).unwrap();
        let second = read_layer(&root, relative, ConfigTier::User, |_| Ok(()))
            .unwrap()
            .unwrap();
        assert_eq!(first.instructions, second.instructions);
        assert_ne!(first.origin.revision, second.origin.revision);
    }

    #[test]
    fn pins_authorized_root_and_rejects_non_regular_config() {
        let temp = tempfile::tempdir().unwrap();
        let container = temp.path().canonicalize().unwrap();
        let root = container.join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("config.toml"), "schema_version = 1").unwrap();
        let layer = read_layer(&root, Path::new("config.toml"), ConfigTier::User, |_| {
            fs::rename(&root, container.join("original")).unwrap();
            fs::create_dir(&root).unwrap();
            fs::write(root.join("config.toml"), "invalid replacement").unwrap();
            Ok(())
        })
        .unwrap()
        .unwrap();
        assert_eq!(layer.instructions, InstructionConfigPatch::default());
        fs::remove_file(root.join("config.toml")).unwrap();
        fs::create_dir(root.join("config.toml")).unwrap();
        assert!(
            read_layer(
                &root,
                Path::new("config.toml"),
                ConfigTier::User,
                |_| Ok(())
            )
            .is_err()
        );
        fs::remove_dir(root.join("config.toml")).unwrap();
        nix::unistd::mkfifo(&root.join("config.toml"), nix::sys::stat::Mode::S_IRUSR).unwrap();
        assert!(
            read_layer(
                &root,
                Path::new("config.toml"),
                ConfigTier::User,
                |_| Ok(())
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_untrusted_malformed_oversized_and_linked_sources() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let relative = Path::new("agent.toml");
        for text in [
            "schema_version = 2",
            "schema_version = 1\ngranted = true",
            "schema_version = 1\n[instructions]\nmax_bytes = 1048577",
            "schema_version = 1\n[instructions]\nfallback_filenames = ['../X']",
            "schema_version = 1\nsecret = 'DO-NOT-REPORT'",
        ] {
            fs::write(root.join(relative), text).unwrap();
            let error = read_layer(&root, relative, ConfigTier::Workspace, |_| Ok(())).unwrap_err();
            assert_eq!(error, ConfigReadError::Invalid);
            assert!(!error.to_string().contains("DO-NOT-REPORT"));
        }
        fs::write(root.join(relative), [0xff]).unwrap();
        assert_eq!(
            read_layer(&root, relative, ConfigTier::Workspace, |_| Err(
                InstructionReadError::Unauthorized
            )),
            Err(ConfigReadError::Read(InstructionReadError::Unauthorized))
        );
        fs::write(root.join(relative), vec![b' '; MAX_CONFIG_BYTES + 1]).unwrap();
        assert_eq!(
            read_layer(&root, relative, ConfigTier::Workspace, |_| Ok(())),
            Err(ConfigReadError::Read(InstructionReadError::Budget))
        );
        fs::remove_file(root.join(relative)).unwrap();
        symlink("elsewhere", root.join(relative)).unwrap();
        assert!(read_layer(&root, relative, ConfigTier::Workspace, |_| Ok(())).is_err());
        symlink("elsewhere", root.join("agent")).unwrap();
        assert!(
            read_layer(
                &root,
                Path::new("agent/config.toml"),
                ConfigTier::User,
                |_| Ok(())
            )
            .is_err()
        );
        assert!(
            read_layer(&root, Path::new("../agent.toml"), ConfigTier::User, |_| Ok(
                ()
            ))
            .is_err()
        );
    }
}
