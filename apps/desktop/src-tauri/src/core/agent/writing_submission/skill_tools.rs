//! Bind frozen skill identities to this manuscript's current native trust checks.
use super::*;
use crate::agent::{
    instructions::InstructionReadError,
    repository::instruction_trust,
    skills::tools::{RegisteredSkill, SkillTools},
};
use std::sync::Arc;

pub(super) async fn for_submission(
    store: AppStore,
    record: SubmissionRecord,
) -> Result<SkillTools, WritingSubmissionError> {
    tokio::task::spawn_blocking(move || {
        let mut registrations = vec![];
        for skill in &record.input.skills {
            if skill.descriptor.identity.environment_id == "application" {
                let root = store
                    .agent_configuration_directory()
                    .canonicalize()
                    .map_err(|_| SubmissionError::Unavailable)?;
                let authorized_root = root.clone();
                registrations.push(RegisteredSkill {
                    descriptor: skill.descriptor.clone(),
                    root,
                    explicit: skill.explicit,
                    authorize: Arc::new(move |identity| {
                        (identity.canonical_path == authorized_root)
                            .then_some(())
                            .ok_or(InstructionReadError::Unauthorized)
                    }),
                });
                continue;
            }
            let manuscript = skill
                .descriptor
                .identity
                .environment_id
                .strip_prefix("manuscript:")
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or(SubmissionError::Corrupt)?;
            validate_scope(&record, manuscript)?;
            // Native preparation currently discovers only this registered source root.
            let source = std::path::Path::new(&skill.descriptor.identity.source_id);
            if !source.starts_with(".agents/skills") {
                return Err(SubmissionError::Corrupt.into());
            }
            let container = store
                .manuscript_directory()
                .canonicalize()
                .map_err(|_| WritingSubmissionError::Unavailable)?;
            let root = container.join(manuscript.to_string()).join("source");
            let database = store.shared_agent_connection();
            let conversation = record.conversation_id;
            registrations.push(RegisteredSkill {
                descriptor: skill.descriptor.clone(),
                root,
                explicit: skill.explicit,
                authorize: Arc::new(move |identity| {
                    let db = database
                        .lock()
                        .map_err(|_| InstructionReadError::Unavailable)?;
                    check_binding(&db, conversation, manuscript)
                        .map_err(|_| InstructionReadError::Unauthorized)?;
                    if instruction_trust::is_trusted(&db, manuscript, identity)
                        .map_err(|_| InstructionReadError::Unavailable)?
                    {
                        Ok(())
                    } else {
                        Err(InstructionReadError::Unauthorized)
                    }
                }),
            });
        }
        SkillTools::new(registrations, 1024 * 1024)
            .map(|tools| tools.with_turn_selections(store, record.turn_id))
            .map_err(|_| SubmissionError::Corrupt.into())
    })
    .await
    .map_err(|_| WritingSubmissionError::Unavailable)?
}
