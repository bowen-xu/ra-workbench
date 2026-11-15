//! Frozen non-file host profiles, independent of evidence and tool permissions.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("instruction profile unavailable")]
    ProfileUnavailable,
    #[error("invalid instruction profile snapshot")]
    InvalidInput,
}
use crate::agent::config::profiles::{ProfileKind, read_user_profile};
use research_agent_core::{
    config::{
        ConfigLayer, ConfigOrigin, ConfigTier, InstructionConfigPatch, InstructionConfigSnapshot,
        resolve_instruction_config,
    },
    instructions::{InstructionDocument, InstructionFragment, assemble_instructions},
    submission::SubmissionInput,
};
use sha2::Digest;
use std::path::Path;

pub(crate) fn freeze(
    root: &Path,
    kind: ProfileKind,
    scope: String,
    overrides: [Option<InstructionConfigPatch>; 2],
) -> Result<(Vec<InstructionFragment>, Vec<InstructionConfigSnapshot>), Error> {
    let Some((profile, revision)) =
        read_user_profile(root, kind).map_err(|_| Error::ProfileUnavailable)?
    else {
        return Ok((vec![], vec![]));
    };
    let mut layers = vec![ConfigLayer {
        origin: ConfigOrigin {
            tier: ConfigTier::User,
            source_id: kind.source_id().into(),
            revision: revision.clone(),
        },
        instructions: InstructionConfigPatch {
            max_bytes: Some(profile.max_bytes),
            fallback_filenames: None,
        },
    }];
    for (tier, patch) in [ConfigTier::Session, ConfigTier::Turn]
        .into_iter()
        .zip(overrides)
    {
        if let Some(instructions) = patch {
            let revision = format!(
                "{:x}",
                sha2::Sha256::digest(
                    serde_json::to_vec(&instructions).map_err(|_| Error::InvalidInput)?
                )
            );
            layers.push(ConfigLayer {
                origin: ConfigOrigin {
                    tier,
                    source_id: format!("profile:{tier:?}"),
                    revision,
                },
                instructions,
            });
        }
    }
    let budget = resolve_instruction_config(&layers)
        .map_err(|_| Error::InvalidInput)?
        .max_bytes
        .value;
    let fragments = assemble_instructions(
        vec![InstructionDocument {
            environment_id: scope.clone(),
            scope_id: scope.clone(),
            source_id: kind.source_id().into(),
            revision: revision.clone(),
            text: profile.text,
        }],
        budget,
    )
    .map_err(|_| Error::ProfileUnavailable)?;
    let config = InstructionConfigSnapshot {
        environment_id: scope,
        layers,
    };
    Ok((fragments, vec![config]))
}

/// Called after the shared submission decoder. Old receipts have no profile;
/// new ones must carry the matching non-file source and its frozen config.
pub(crate) fn validate(
    input: &SubmissionInput,
    kind: ProfileKind,
    scope: &str,
) -> Result<(), Error> {
    if input.instructions.is_empty() && input.instruction_configs.is_empty() {
        return Ok(());
    }
    let [fragment] = input.instructions.as_slice() else {
        return Err(Error::InvalidInput);
    };
    let [config] = input.instruction_configs.as_slice() else {
        return Err(Error::InvalidInput);
    };
    let Some(layer) = config.layers.first() else {
        return Err(Error::InvalidInput);
    };
    for override_layer in &config.layers[1..] {
        if !matches!(
            override_layer.origin.tier,
            ConfigTier::Session | ConfigTier::Turn
        ) || override_layer.origin.source_id
            != format!("profile:{:?}", override_layer.origin.tier)
            || override_layer.origin.revision
                != format!(
                    "{:x}",
                    sha2::Sha256::digest(
                        serde_json::to_vec(&override_layer.instructions)
                            .map_err(|_| Error::InvalidInput)?
                    )
                )
        {
            return Err(Error::InvalidInput);
        }
    }
    let budget = resolve_instruction_config(&config.layers)
        .map_err(|_| Error::InvalidInput)?
        .max_bytes
        .value;
    if fragment.environment_id != scope
        || fragment.scope_id != scope
        || fragment.source_id != kind.source_id()
        || config.environment_id != scope
        || layer.origin.tier != ConfigTier::User
        || layer.origin.source_id != kind.source_id()
        || layer.origin.revision != fragment.revision
        || fragment.revision.len() != 64
        || !fragment
            .revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || layer.instructions.fallback_filenames.is_some()
        || fragment.original_bytes > 65536
        || layer.instructions.max_bytes.is_none()
        || fragment.text.len() > budget
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn frozen_overrides_restore_with_effective_budget_and_verified_provenance() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("agent")).unwrap();
        fs::write(root.path().join("agent/config.toml"),
            "schema_version=1\n[profiles.reading]\ntext='中文'\nmax_bytes=1\n[profiles.note]\ntext='中文'\nmax_bytes=1\n[profiles.reading_note]\ntext='中文'\nmax_bytes=1\n").unwrap();
        for (kind, scope) in [
            (ProfileKind::Reading, "paper:id"),
            (ProfileKind::Note, "note:id"),
            (ProfileKind::ReadingNote, "reading-note:id"),
        ] {
            for limit in [Some(0), Some(3), None] {
                let (instructions, instruction_configs) = freeze(
                    root.path(),
                    kind,
                    scope.into(),
                    [
                        Some(InstructionConfigPatch {
                            max_bytes: Some(6),
                            fallback_filenames: Some(vec![]),
                        }),
                        Some(InstructionConfigPatch {
                            max_bytes: limit,
                            fallback_filenames: None,
                        }),
                    ],
                )
                .unwrap();
                assert_eq!(
                    instructions[0].text,
                    match limit {
                        Some(0) => "",
                        Some(3) => "中",
                        _ => "中文",
                    }
                );
                let input = SubmissionInput {
                    remote_compaction_v2: None,
                    user_input: vec![],
                    skill_instructions: vec![],
                    text: "question".into(),
                    model: research_agent_core::protocol::ModelIdentity {
                        provider_id: "fixture".into(),
                        model_id: "fixture".into(),
                        revision: None,
                    },
                    reasoning_effort: None,
                    service_tier: None,
                    context: vec![],
                    instructions,
                    instruction_configs,
                    skills: vec![],
                };
                let input: SubmissionInput =
                    serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
                assert!(validate(&input, kind, scope).is_ok());
                assert!(validate(&input, kind, "foreign:id").is_err());
                let mut invalid = input.clone();
                invalid.instruction_configs[0].layers[1].origin.source_id = "profile:Turn".into();
                assert!(validate(&invalid, kind, scope).is_err());
                let mut invalid = input.clone();
                invalid.instruction_configs[0].layers[1]
                    .instructions
                    .max_bytes = Some(7);
                assert!(validate(&invalid, kind, scope).is_err());
                let mut invalid = input.clone();
                invalid.instruction_configs[0].layers.swap(1, 2);
                assert!(validate(&invalid, kind, scope).is_err());
                let mut invalid = input.clone();
                invalid.instruction_configs[0]
                    .layers
                    .push(input.instruction_configs[0].layers[2].clone());
                assert!(validate(&invalid, kind, scope).is_err());
            }
        }
    }

    #[test]
    fn freezes_each_non_file_profile_with_its_host_scope() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("agent")).unwrap();
        fs::write(
            root.path().join("agent/config.toml"),
            "schema_version=1\n[profiles.reading]\ntext='Reading rules'\nmax_bytes=7\n",
        )
        .unwrap();
        let (fragments, configs) = freeze(
            root.path(),
            ProfileKind::Reading,
            "paper:paper-1".into(),
            [None, None],
        )
        .unwrap();
        assert_eq!(fragments[0].environment_id, "paper:paper-1");
        assert_eq!(fragments[0].text, "Reading");
        assert_eq!(configs[0].layers[0].origin.tier, ConfigTier::User);
        assert!(
            validate(
                &SubmissionInput {
                    remote_compaction_v2: None,
                    user_input: vec![],
                    skill_instructions: vec![],
                    text: "question".into(),
                    model: research_agent_core::protocol::ModelIdentity {
                        provider_id: "fixture".into(),
                        model_id: "fixture".into(),
                        revision: None,
                    },
                    reasoning_effort: None,
                    service_tier: None,
                    context: vec![],
                    instructions: fragments,
                    instruction_configs: configs,
                    skills: vec![],
                },
                ProfileKind::Reading,
                "paper:paper-1",
            )
            .is_ok()
        );
    }
}
