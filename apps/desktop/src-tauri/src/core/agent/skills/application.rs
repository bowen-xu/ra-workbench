//! Application-scoped skills shared by non-workspace Agent hosts.
//!
//! This reuses the Codex-derived discovery/load split. Reading and note
//! submissions do not own a manuscript trust root, so their skills are
//! resolved only from the app-owned configuration directory and are frozen
//! into the submission before a model request can start.

use super::{
    discovery::discover_under,
    tools::{RegisteredSkill, SkillTools},
};
use crate::{agent::instructions::InstructionReadError, storage::AppStore};
use research_agent_core::{
    skills::selection::{select_explicit, select_explicit_sources},
    submission::{SubmissionError, SubmissionInput, SubmissionRecord},
};
use std::{path::Path, sync::Arc};

const ENVIRONMENT: &str = "application";
const SKILL_ROOT: &str = ".agents/skills";

pub(crate) fn discover(
    configuration_directory: &Path,
) -> Result<super::discovery::SkillDiscovery, InstructionReadError> {
    if !configuration_directory.exists() {
        return Ok(super::discovery::SkillDiscovery {
            skills: Vec::new(),
            rejected: Vec::new(),
        });
    }
    let root = configuration_directory
        .canonicalize()
        .map_err(|_| InstructionReadError::Unavailable)?;
    let mut discovery = discover_under(&root, Path::new(SKILL_ROOT), ENVIRONMENT, 8, |identity| {
        (identity.canonical_path == root)
            .then_some(())
            .ok_or(InstructionReadError::Unauthorized)
    })
    .map_err(|error| match error {
        super::files::SkillReadError::Read(error) => error,
        _ => InstructionReadError::InvalidScope,
    })?;
    for skill in &mut discovery.skills {
        *skill = super::files::read_descriptor(
            &root,
            Path::new(&skill.identity.source_id),
            ENVIRONMENT,
            |_| Ok(()),
        )
        .map_err(|_| InstructionReadError::InvalidScope)?;
    }
    Ok(discovery)
}

pub(crate) fn validate(input: &SubmissionInput) -> Result<(), SubmissionError> {
    for skill in &input.skills {
        if skill.descriptor.identity.environment_id != ENVIRONMENT
            || !Path::new(&skill.descriptor.identity.source_id).starts_with(SKILL_ROOT)
        {
            return Err(SubmissionError::Corrupt);
        }
    }
    Ok(())
}

pub(crate) fn freeze(
    configuration_directory: &Path,
    user_text: &str,
) -> Result<Vec<research_agent_core::skills::SubmissionSkill>, InstructionReadError> {
    let catalog = discover(configuration_directory)?;
    if !catalog.rejected.is_empty() {
        return Err(InstructionReadError::InvalidScope);
    }
    let rules =
        crate::agent::config::skills::read(configuration_directory).map_err(
            |error| match error {
                crate::agent::config::ConfigReadError::Read(error) => error,
                crate::agent::config::ConfigReadError::Invalid => {
                    InstructionReadError::InvalidScope
                }
            },
        )?;
    select_explicit(
        catalog
            .skills
            .into_iter()
            .filter(|skill| rules.enabled(&skill.identity.source_id))
            .collect(),
        user_text,
    )
    .map_err(|_| InstructionReadError::InvalidScope)
}

/// Freeze only the caller's explicit source IDs. This is the structured path
/// used by typed workflows; prompt text is never interpreted as a selector.
pub(crate) fn freeze_sources(
    configuration_directory: &Path,
    source_ids: &[String],
) -> Result<Vec<research_agent_core::skills::SubmissionSkill>, InstructionReadError> {
    let catalog = discover(configuration_directory)?;
    if !catalog.rejected.is_empty() {
        return Err(InstructionReadError::InvalidScope);
    }
    let rules =
        crate::agent::config::skills::read(configuration_directory).map_err(
            |error| match error {
                crate::agent::config::ConfigReadError::Read(error) => error,
                crate::agent::config::ConfigReadError::Invalid => {
                    InstructionReadError::InvalidScope
                }
            },
        )?;
    select_explicit_sources(
        catalog
            .skills
            .into_iter()
            .filter(|skill| rules.enabled(&skill.identity.source_id))
            .collect(),
        source_ids,
    )
    .map_err(|_| InstructionReadError::InvalidScope)
}

pub(crate) async fn tools(
    store: AppStore,
    record: SubmissionRecord,
) -> Result<SkillTools, SubmissionError> {
    tokio::task::spawn_blocking(move || {
        if record.input.skills.is_empty() {
            return SkillTools::new(Vec::new(), 1024 * 1024).map_err(|_| SubmissionError::Corrupt);
        }
        let root = store
            .agent_configuration_directory()
            .canonicalize()
            .map_err(|_| SubmissionError::Unavailable)?;
        let mut registrations = Vec::with_capacity(record.input.skills.len());
        for skill in &record.input.skills {
            if skill.descriptor.identity.environment_id != ENVIRONMENT {
                return Err(SubmissionError::Corrupt);
            }
            let source = Path::new(&skill.descriptor.identity.source_id);
            if !source.starts_with(SKILL_ROOT) {
                return Err(SubmissionError::Corrupt);
            }
            let root_for_auth = root.clone();
            registrations.push(RegisteredSkill {
                descriptor: skill.descriptor.clone(),
                root: root.clone(),
                explicit: skill.explicit,
                authorize: Arc::new(move |identity| {
                    (identity.canonical_path == root_for_auth)
                        .then_some(())
                        .ok_or(InstructionReadError::Unauthorized)
                }),
            });
        }
        SkillTools::new(registrations, 1024 * 1024)
            .map(|tools| tools.with_turn_selections(store, record.turn_id))
            .map_err(|_| SubmissionError::Corrupt)
    })
    .await
    .map_err(|_| SubmissionError::Unavailable)?
}

/// Compose the same frozen, on-demand skill owner for every non-workspace host.
pub(crate) async fn registration(
    store: AppStore,
    record: SubmissionRecord,
    previous: research_agent_core::tool_registry::OwnedToolRegistration,
) -> Result<research_agent_core::tool_registry::OwnedToolRegistration, SubmissionError> {
    use crate::agent::composed_tools::ComposedToolExecutor;
    use research_agent_core::tool_registry::OwnedToolRegistration;
    let skills = Arc::new(tools(store, record).await?);
    let own = skills.definitions();
    let (mut definitions, executor) = previous.into_parts();
    definitions.retain(|definition| {
        !own.iter()
            .any(|skill| skill.name == definition.name && skill.namespace == definition.namespace)
    });
    let fallback = definitions.clone();
    definitions.extend(own.clone());
    Ok(OwnedToolRegistration::new(
        definitions,
        Arc::new(ComposedToolExecutor::new(
            skills,
            own,
            Some(executor),
            fallback,
        )),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn general_host_loads_frozen_skill_and_reference_only_on_demand_after_restore() {
        use research_agent_core::{
            model::ToolCall, protocol::ModelIdentity, tool_registry::OwnedToolRegistration,
        };
        let directory = tempfile::tempdir().unwrap();
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let conversation = uuid::Uuid::new_v4();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'General skill fixture')",
            [conversation.to_string()],
        )
        .unwrap();
        let store = AppStore::for_test(db, directory.path().join("objects"));
        let package = store
            .agent_configuration_directory()
            .join(".agents/skills/editing");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("SKILL.md"),
            "---\ndescription: Edit structured objects\n---\nRead schema.md before editing.",
        )
        .unwrap();
        fs::write(
            package.join("schema.md"),
            "Synthetic detailed schema, loaded only when requested.",
        )
        .unwrap();
        let (skills, prompts) = super::super::injection::freeze_application(
            store.agent_configuration_directory(),
            "Discuss",
            &[],
        )
        .unwrap();
        assert!(prompts.is_empty());
        assert!(!skills[0].explicit);
        let identity = skills[0].descriptor.identity.clone();
        let record = {
            let mut db = store.connection().unwrap();
            let tx = db.transaction().unwrap();
            let record = crate::agent::repository::submissions::insert_in(
                &tx,
                conversation,
                uuid::Uuid::new_v4(),
                SubmissionInput {
                    remote_compaction_v2: None,
                    text: "Discuss".into(),
                    user_input: vec![],
                    skill_instructions: prompts,
                    model: ModelIdentity {
                        provider_id: "fixture".into(),
                        model_id: "fixture".into(),
                        revision: None,
                    },
                    reasoning_effort: None,
                    service_tier: None,
                    context: vec![],
                    instructions: vec![],
                    instruction_configs: vec![],
                    skills,
                },
            )
            .unwrap();
            tx.commit().unwrap();
            record
        };
        for _ in 0..2 {
            let tools = registration(
                store.clone(),
                record.clone(),
                OwnedToolRegistration::new(
                    vec![],
                    Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
                ),
            )
            .await
            .unwrap();
            let (definitions, executor) = tools.into_parts();
            assert_eq!(
                definitions
                    .iter()
                    .filter(|definition| definition.name == "research_load_skill")
                    .count(),
                1
            );
            assert!(definitions.iter().all(|definition| {
                !definition.description.contains("Synthetic detailed schema")
                    && !definition
                        .parameters
                        .to_string()
                        .contains("Synthetic detailed schema")
            }));
            let loaded = executor
                .authorize_and_execute(ToolCall {
                    namespace: None,
                    raw_arguments_json: None,
                    call_id: "load".into(),
                    name: "research_load_skill".into(),
                    arguments: serde_json::json!({"identity":identity}),
                })
                .await
                .unwrap();
            assert!(!loaded.is_error);
            assert!(loaded.content.contains("Read schema.md"));
            let reference=executor.authorize_and_execute(ToolCall{
namespace: None,
raw_arguments_json: None,call_id:"schema".into(),name:"research_read_skill_resource".into(),arguments:serde_json::json!({"identity":identity,"path":"schema.md","expected_revision":null})}).await.unwrap();
            assert!(!reference.is_error);
            assert!(reference.content.contains("Synthetic detailed schema"));
        }
    }

    #[test]
    fn freezes_only_explicit_skills_from_the_application_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let skill = root.join(SKILL_ROOT).join("review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\ndescription: Review prose\n---\nReview body",
        )
        .unwrap();
        let selected = freeze(&root, "$review").unwrap();
        assert_eq!(selected.len(), 1);
        assert!(selected[0].explicit);
        assert_eq!(selected[0].descriptor.identity.environment_id, ENVIRONMENT);
    }

    #[test]
    fn rejects_persisted_skill_identities_outside_the_application_root() {
        let input = SubmissionInput {
            remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
            text: "review".into(),
            model: research_agent_core::protocol::ModelIdentity {
                provider_id: "test".into(),
                model_id: "test".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![research_agent_core::skills::SubmissionSkill {
                descriptor: research_agent_core::skills::SkillDescriptor {
                    identity: research_agent_core::skills::SkillIdentity {
                        environment_id: "manuscript:other".into(),
                        source_id: ".agents/skills/review/SKILL.md".into(),
                        revision: "revision".into(),
                    },
                    name: "review".into(),
                    description: "Review".into(),
                    allow_implicit_invocation: true,
                },
                explicit: true,
            }],
        };
        assert_eq!(validate(&input), Err(SubmissionError::Corrupt));
    }

    #[test]
    fn disabled_application_skill_is_not_frozen_even_when_explicitly_named() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let skill = root.join(SKILL_ROOT).join("review");
        fs::create_dir_all(&skill).unwrap();
        fs::create_dir(root.join("agent")).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\ndescription: Review prose\n---\nReview body",
        )
        .unwrap();
        fs::create_dir_all(root.join("agent")).unwrap();
        fs::write(
            root.join("agent/config.toml"),
            "schema_version=1\n[[skills.config]]\nsource_id='.agents/skills/review/SKILL.md'\nenabled=false",
        )
        .unwrap();
        assert!(freeze(&root, "$review").unwrap().is_empty());
    }

    #[test]
    fn structured_source_selection_ignores_unknown_and_disabled_sources() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let skill = root.join(SKILL_ROOT).join("review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\ndescription: Review prose\n---\nReview body",
        )
        .unwrap();
        fs::create_dir_all(root.join("agent")).unwrap();
        fs::write(
            root.join("agent/config.toml"),
            "schema_version=1\n[[skills.config]]\nsource_id='.agents/skills/review/SKILL.md'\nenabled=false",
        )
        .unwrap();
        let selected = freeze_sources(
            &root,
            &[
                ".agents/skills/review/SKILL.md".to_owned(),
                ".agents/skills/missing/SKILL.md".to_owned(),
            ],
        )
        .unwrap();
        assert!(selected.is_empty());
    }
}
