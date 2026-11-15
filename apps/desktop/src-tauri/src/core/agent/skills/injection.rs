//! Adapted from Codex core-skills/src/injection.rs at
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b (Apache-2.0).
//! Copyright 2025 OpenAI. See third-party/codex/LICENSE and NOTICE.
//! This adapter supplies trusted native roots and persists loaded prompts.
use super::files::load_selected;
use crate::{
    agent::instructions::{InstructionReadError, InstructionRootIdentity},
    storage::AppStore,
};
use research_agent_core::{
    skills::{SubmissionSkill, instructions::SkillInstructions, selection::select_inputs},
    submission::{ContextText, SubmissionError},
    user_input::{MentionTarget, UserInput},
};
use rusqlite::Connection;
use std::path::Path;

pub(crate) fn load_prompts(
    root: &Path,
    skills: &[SubmissionSkill],
    authorize: impl Fn(&InstructionRootIdentity<'_>) -> Result<(), InstructionReadError>,
) -> Result<Vec<SkillInstructions>, InstructionReadError> {
    let mut remaining = 1_048_576usize;
    let mut prompts = Vec::new();
    for skill in skills.iter().filter(|skill| skill.explicit) {
        let loaded = load_selected(root, &skill.descriptor, true, remaining, &authorize)
            .map_err(|_| InstructionReadError::InvalidScope)?;
        let contents = frozen_contents(&loaded.body, &loaded.identity);
        remaining = remaining
            .checked_sub(contents.len())
            .ok_or(InstructionReadError::Budget)?;
        prompts.push(SkillInstructions {
            identity: loaded.identity,
            name: skill.descriptor.name.clone(),
            contents,
        });
    }
    Ok(prompts)
}

pub(crate) fn frozen_contents(
    body: &str,
    identity: &research_agent_core::skills::SkillIdentity,
) -> String {
    format!(
        "{}\n\nRead-only skill package, when shell is authorized: {}. Run scripts only through the registered shell tools. Report missing dependencies; do not install them implicitly.",
        body,
        super::packages::relative(identity)
    )
}

pub(crate) fn freeze_application(
    root: &Path,
    text: &str,
    input: &[UserInput],
) -> Result<(Vec<SubmissionSkill>, Vec<SkillInstructions>), InstructionReadError> {
    research_agent_core::user_input::validate(input, text)
        .map_err(|_| InstructionReadError::InvalidScope)?;
    let catalog = super::application::freeze(root, text)?;
    // Workspace items are resolved independently against the workspace catalog.
    let application_items: Vec<_> = input.iter().filter(|item| matches!(item, UserInput::Skill { identity, .. } if identity.environment_id == "application")).cloned().collect();
    let mut skills = select_inputs(
        catalog.into_iter().map(|skill| skill.descriptor).collect(),
        text,
        &application_items,
    )
    .map_err(|_| InstructionReadError::InvalidScope)?;
    if skills.iter().any(|skill| skill.explicit) {
        let root = root
            .canonicalize()
            .map_err(|_| InstructionReadError::Unavailable)?;
        let prompts = load_prompts(&root, &skills, |identity| {
            (identity.canonical_path == root)
                .then_some(())
                .ok_or(InstructionReadError::Unauthorized)
        })?;
        Ok((skills, prompts))
    } else {
        Ok((std::mem::take(&mut skills), Vec::new()))
    }
}

/// Resolve known tool configuration; unresolved tags remain ordinary text.
pub(crate) fn validate_mentions(
    store: &AppStore,
    db: &Connection,
    input: &[UserInput],
    _context: &[ContextText],
) -> Result<(), SubmissionError> {
    for item in input {
        if let UserInput::Mention { target, .. } = item {
            match target {
                // Access comes from the selected live context. Missing file tags
                // remain ordinary message text and cannot block submission.
                MentionTarget::File { .. } => {}
                MentionTarget::Tool {
                    server_id,
                    environment_id,
                    revision,
                } => {
                    let servers = crate::agent::mcp_config::read_from_root(
                        store.agent_configuration_directory(),
                    )
                    .map_err(|_| SubmissionError::Unavailable)?
                    .unwrap_or_default();
                    let Some(server) = servers.iter().find(|server| {
                        &server.server_id == server_id
                            && &server.environment_id == environment_id
                            && &server.revision == revision
                            && server.enabled
                    }) else {
                        continue;
                    };
                    let approved = crate::agent::repository::mcp_approvals::read(db, server_id)
                        .map_err(|_| SubmissionError::Unavailable)?;
                    if !approved.is_some_and(|receipt| {
                        receipt.environment_id == server.environment_id
                            && receipt.revision == server.revision
                    }) {
                        continue;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn explicit_skill_load_is_version_bound_and_budgeted() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join(".agents/skills/review");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            "---\nname: review\ndescription: Review prose\n---\nUse the references.",
        )
        .unwrap();
        let descriptor = super::super::application::discover(temp.path())
            .unwrap()
            .skills
            .remove(0);
        let items = vec![
            UserInput::Text {
                text: "@review".into(),
                text_elements: vec![],
            },
            UserInput::Skill {
                name: descriptor.name.clone(),
                identity: descriptor.identity.clone(),
            },
        ];
        let (skills, prompts) = freeze_application(temp.path(), "@review", &items).unwrap();
        assert!(skills[0].explicit);
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].render().contains("Use the references."));
        assert!(
            freeze_application(temp.path(), "ordinary message", &[])
                .unwrap()
                .1
                .is_empty()
        );
        fs::write(
            directory.join("SKILL.md"),
            "---\ndescription: Changed\n---\nNew version",
        )
        .unwrap();
        assert!(
            freeze_application(temp.path(), "@review", &items)
                .unwrap()
                .1
                .is_empty()
        );
        fs::write(
            directory.join("SKILL.md"),
            format!("---\ndescription: Review\n---\n{}", "a".repeat(1_048_577)),
        )
        .unwrap();
        assert!(freeze_application(temp.path(), "$review", &[]).is_err());
    }
}
