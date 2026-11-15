//! Materialize trusted instructions for the already-validated source selection.
use crate::agent::{
    instructions::{InstructionReadError, read_project_instruction_scopes_with_fallbacks},
    repository::instruction_trust,
    writing_resources::SourceDirectory,
};
use research_agent_core::{
    config::{
        ConfigLayer, ConfigOrigin, ConfigTier, DEFAULT_INSTRUCTION_BYTES, InstructionConfigPatch,
        InstructionConfigSnapshot, resolve_instruction_config,
    },
    instructions::{InstructionFragment, assemble_instructions},
    skills::SubmissionSkill,
    submission::ContextText,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct TrustedContext {
    pub(super) instructions: Vec<InstructionFragment>,
    pub(super) instruction_configs: Vec<InstructionConfigSnapshot>,
    pub(super) skills: Vec<SubmissionSkill>,
}

/// Pure input selected at the original Core preparation call site.
/// The source is required only after the original trust checks succeed.
pub(super) struct InstructionEnvironment<'a> {
    pub(super) manuscripts: &'a Path,
    pub(super) user_configuration: &'a Path,
    pub(super) source_directory: Option<SourceDirectory>,
}

/// Caller owns the submission transaction and runs on a blocking worker.
/// Untrusted manuscripts do not probe instruction files. Trust does not grant
/// provider disclosure: returned content belongs in the confirmation snapshot.
pub(super) fn materialize_in(
    db: &Connection,
    environment: InstructionEnvironment<'_>,
    manuscript: Uuid,
    sources: &[ContextText],
    user_text: &str,
    overrides: [Option<InstructionConfigPatch>; 2],
) -> Result<TrustedContext, InstructionReadError> {
    let InstructionEnvironment {
        manuscripts,
        user_configuration,
        source_directory,
    } = environment;
    let mut override_layers = Vec::new();
    for (tier, patch) in [ConfigTier::Session, ConfigTier::Turn]
        .into_iter()
        .zip(overrides)
    {
        if let Some(instructions) = patch {
            let encoded = serde_json::to_vec(&instructions)
                .map_err(|_| InstructionReadError::InvalidScope)?;
            override_layers.push(ConfigLayer {
                origin: ConfigOrigin {
                    tier,
                    source_id: if tier == ConfigTier::Session {
                        "submission:session"
                    } else {
                        "submission:turn"
                    }
                    .into(),
                    revision: format!("{:x}", Sha256::digest(encoded)),
                },
                instructions,
            });
        }
    }
    resolve_instruction_config(&override_layers).map_err(|_| InstructionReadError::InvalidScope)?;
    let trust =
        instruction_trust::read(db, manuscript).map_err(|_| InstructionReadError::Unavailable)?;
    if !trust.is_some_and(|state| state.granted) {
        if !override_layers.is_empty() {
            return Err(InstructionReadError::Unauthorized);
        }
        return Ok(TrustedContext::default());
    }
    let source_directory = source_directory.ok_or(InstructionReadError::Unavailable)?;
    let root = source_directory(manuscripts, &manuscript.to_string())
        .map_err(|_| InstructionReadError::InvalidScope)?;
    // Canonicalize only the app-owned container, not the manuscript/source links.
    let container = manuscripts
        .canonicalize()
        .map_err(|_| InstructionReadError::Unavailable)?;
    let relative = root
        .strip_prefix(manuscripts)
        .map_err(|_| InstructionReadError::InvalidScope)?;
    let root = container.join(relative);
    let mut scopes: Vec<_> = sources
        .iter()
        .map(|source| {
            Path::new(&source.resource_id)
                .parent()
                .unwrap_or(Path::new(""))
        })
        .collect();
    scopes.sort();
    scopes.dedup();
    let authorize = |identity: &crate::agent::instructions::InstructionRootIdentity<'_>| {
        if instruction_trust::is_trusted(db, manuscript, identity)
            .map_err(|_| InstructionReadError::Unavailable)?
        {
            Ok(())
        } else {
            Err(InstructionReadError::Unauthorized)
        }
    };
    let config_path = Path::new(".research-assistant/agent.toml");
    let mut config_layers = Vec::new();
    let mut directories = vec![PathBuf::new()];
    for scope in &scopes {
        let mut current = PathBuf::new();
        for component in scope.components() {
            current.push(component.as_os_str());
            directories.push(current.clone());
        }
    }
    directories.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    directories.dedup();
    for (index, directory) in directories.iter().enumerate() {
        let relative = if directory.as_os_str().is_empty() {
            config_path.to_path_buf()
        } else {
            directory.join(config_path)
        };
        let tier = if index == 0 {
            ConfigTier::Workspace
        } else {
            ConfigTier::Directory
        };
        if let Some(layer) =
            crate::agent::config::read_layer(&root, &relative, tier, |identity| authorize(identity))
                .map_err(|error| match error {
                    crate::agent::config::ConfigReadError::Read(error) => error,
                    crate::agent::config::ConfigReadError::Invalid => {
                        InstructionReadError::InvalidScope
                    }
                })?
        {
            config_layers.push((directory.clone(), layer));
        }
    }
    let mut layers = vec![ConfigLayer {
        origin: ConfigOrigin {
            tier: ConfigTier::Application,
            source_id: "builtin:instructions".into(),
            revision: "1".into(),
        },
        instructions: InstructionConfigPatch {
            max_bytes: Some(DEFAULT_INSTRUCTION_BYTES),
            fallback_filenames: Some(vec![]),
        },
    }];
    layers.extend(
        crate::agent::config::read_user_layer(user_configuration).map_err(|error| match error {
            crate::agent::config::ConfigReadError::Read(error) => error,
            crate::agent::config::ConfigReadError::Invalid => InstructionReadError::InvalidScope,
        })?,
    );
    let environment_id = format!("manuscript:{manuscript}");
    // A Codex config stack describes ONE root-to-cwd chain. Resolve each
    // directory against its own ancestors, never against a sibling's settings.
    let mut snapshots = Vec::new();
    let mut configs = std::collections::BTreeMap::new();
    for directory in &directories {
        let mut scope_layers = layers.clone();
        scope_layers.extend(
            config_layers
                .iter()
                .filter(|(ancestor, _)| directory.starts_with(ancestor))
                .map(|(_, layer)| layer.clone()),
        );
        scope_layers.extend(override_layers.clone());
        let config = resolve_instruction_config(&scope_layers)
            .map_err(|_| InstructionReadError::InvalidScope)?;
        configs.insert(directory.clone(), config);
        snapshots.push(InstructionConfigSnapshot {
            environment_id: if directory.as_os_str().is_empty() {
                environment_id.clone()
            } else {
                format!(
                    "{environment_id}/directory:{}",
                    directory
                        .to_str()
                        .ok_or(InstructionReadError::InvalidScope)?
                )
            },
            layers: scope_layers,
        });
    }
    research_agent_core::config::validate_snapshots(&snapshots)
        .map_err(|_| InstructionReadError::Budget)?;
    let documents = read_project_instruction_scopes_with_fallbacks(
        &root,
        &scopes,
        &environment_id,
        |scope| {
            configs
                .get(scope)
                .map(|config| config.fallback_filenames.value.clone())
                .ok_or(InstructionReadError::InvalidScope)
        },
        authorize,
    )?;
    // Each chain's budget includes its ancestors. The union is additionally
    // capped by the largest resolved budget, so selecting siblings cannot
    // multiply the submission's instruction allowance.
    let mut remaining = configs
        .values()
        .map(|config| config.max_bytes.value)
        .max()
        .unwrap_or(0);
    let mut consumed = Vec::<(PathBuf, usize)>::new();
    let mut instructions = Vec::new();
    for document in documents {
        let scope = if document.scope_id == "." {
            PathBuf::new()
        } else {
            PathBuf::from(&document.scope_id)
        };
        let config = configs
            .get(&scope)
            .ok_or(InstructionReadError::InvalidScope)?;
        let ancestor_bytes: usize = consumed
            .iter()
            .filter(|(ancestor, _)| scope.starts_with(ancestor))
            .map(|(_, bytes)| bytes)
            .sum();
        let budget = config
            .max_bytes
            .value
            .saturating_sub(ancestor_bytes)
            .min(remaining);
        let original_bytes = document.text.len();
        let fragments = assemble_instructions(vec![document], budget)
            .map_err(|_| InstructionReadError::InvalidScope)?;
        remaining = remaining.saturating_sub(original_bytes.min(budget));
        consumed.push((scope, original_bytes));
        instructions.extend(fragments);
    }
    let catalog = crate::agent::skills::discovery::discover_under(
        &root,
        Path::new(".agents/skills"),
        &environment_id,
        8,
        authorize,
    )
    .map_err(|_| InstructionReadError::InvalidScope)?;
    if !catalog.rejected.is_empty() {
        return Err(InstructionReadError::InvalidScope);
    }
    let skills = research_agent_core::skills::selection::select_explicit(catalog.skills, user_text)
        .map_err(|_| InstructionReadError::InvalidScope)?;
    Ok(TrustedContext {
        instructions,
        instruction_configs: snapshots,
        skills,
    })
}
