//! Same selection/injection as a new turn, bounded by its frozen catalog and grants.
use crate::{
    agent::{
        instructions::InstructionReadError,
        repository::{instruction_trust, submissions},
    },
    storage::AppStore,
};
use research_agent_core::{
    steering::{SteeringError, SteeringInput},
    submission::SubmissionRecord,
    user_input::{MentionTarget, UserInput},
};
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn freeze_grants(
    db: &Connection,
    store: &AppStore,
    record: &SubmissionRecord,
) -> Result<(), SteeringError> {
    freeze_grants_sql(db, store, record)
        .map_err(|cause| cause.domain_or(SteeringError::Unavailable))
}

pub(crate) fn freeze_grants_sql(
    db: &Connection,
    store: &AppStore,
    record: &SubmissionRecord,
) -> Result<(), crate::agent::repository::SqlRepositoryError<SteeringError>> {
    use crate::agent::repository::SqlRepositoryError as E;
    let frozen: bool = db
        .query_row(
            "select exists(select 1 from agent_turn_resource_grants where turn_id=?1)",
            [record.turn_id.to_string()],
            |row| row.get(0),
        )
        .map_err(E::Sql)?;
    if frozen {
        return Ok(());
    }
    crate::agent::host_execution::freeze_grant_sql(db, record).map_err(E::Sql)?;
    let servers = crate::agent::mcp_config::read_from_root(store.agent_configuration_directory())
        .map_err(|_| E::Domain(SteeringError::Unavailable))?
        .unwrap_or_default();
    let mut grants = Vec::new();
    for server in servers.iter().filter(|server| server.enabled) {
        if crate::agent::repository::mcp_approvals::read_sql(db, &server.server_id)
            .map_err(|cause| match cause {
                E::Sql(cause) => E::Sql(cause),
                E::Domain(_) => E::Domain(SteeringError::Unavailable),
            })?
            .is_some_and(|receipt| {
                receipt.environment_id == server.environment_id
                    && receipt.revision == server.revision
            })
        {
            grants.push(MentionTarget::Tool {
                server_id: server.server_id.clone(),
                environment_id: server.environment_id.clone(),
                revision: server.revision.clone(),
            });
        }
    }
    db.execute(
        "insert or ignore into agent_turn_resource_grants(turn_id,grants_json) values(?1,?2)",
        params![
            record.turn_id.to_string(),
            serde_json::to_string(&grants).map_err(|_| E::Domain(SteeringError::Invalid))?
        ],
    )
    .map_err(E::Sql)?;
    Ok(())
}

pub(crate) fn prepare(
    db: &Connection,
    store: &AppStore,
    input: &mut SteeringInput,
) -> Result<(), SteeringError> {
    if input.user_input.is_empty() {
        return Ok(());
    }
    let request: String = db
        .query_row(
            "select request_id from agent_submissions where conversation_id=?1 and turn_id=?2",
            params![
                input.scope.conversation_id.to_string(),
                input.scope.turn_id.to_string()
            ],
            |row| row.get(0),
        )
        .map_err(|_| SteeringError::Closed)?;
    let request = uuid::Uuid::parse_str(&request).map_err(|_| SteeringError::Invalid)?;
    let record = submissions::read(db, input.scope.conversation_id, request)
        .map_err(|_| SteeringError::Unavailable)?
        .ok_or(SteeringError::Closed)?;
    super::injection::validate_mentions(store, db, &input.user_input, &record.input.context)
        .map_err(|_| SteeringError::Conflict)?;
    let grants_json: Option<String> = db
        .query_row(
            "select grants_json from agent_turn_resource_grants where turn_id=?1",
            [input.scope.turn_id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| SteeringError::Unavailable)?;
    let grants: Vec<MentionTarget> = grants_json
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|_| SteeringError::Invalid)?
        .unwrap_or_default();
    for reference in &input.user_input {
        match reference {
            UserInput::Mention {
                target: target @ MentionTarget::Tool { .. },
                ..
            } if !grants.contains(target) => continue,
            UserInput::Skill { name, identity } => {
                let Some(mut skill) = record
                    .input
                    .skills
                    .iter()
                    .find(|skill| {
                        &skill.descriptor.identity == identity && &skill.descriptor.name == name
                    })
                    .cloned()
                else {
                    continue;
                };
                skill.explicit = true;
                if input
                    .skill_instructions
                    .iter()
                    .any(|prompt| &prompt.identity == identity)
                {
                    continue;
                }
                let prompts = if identity.environment_id == "application" {
                    let catalog =
                        super::application::freeze(store.agent_configuration_directory(), "")
                            .map_err(|_| SteeringError::Conflict)?;
                    if !catalog.iter().any(|current| {
                        current.descriptor.identity == *identity && current.descriptor.name == *name
                    }) {
                        continue;
                    }
                    let root = store
                        .agent_configuration_directory()
                        .canonicalize()
                        .map_err(|_| SteeringError::Unavailable)?;
                    super::injection::load_prompts(&root, &[skill], |opened| {
                        (opened.canonical_path == root)
                            .then_some(())
                            .ok_or(InstructionReadError::Unauthorized)
                    })
                } else {
                    let manuscript = identity
                        .environment_id
                        .strip_prefix("manuscript:")
                        .and_then(|id| uuid::Uuid::parse_str(id).ok())
                        .ok_or(SteeringError::Conflict)?;
                    let root = store
                        .manuscript_directory()
                        .join(manuscript.to_string())
                        .join("source")
                        .canonicalize()
                        .map_err(|_| SteeringError::Unavailable)?;
                    super::injection::load_prompts(&root, &[skill], |opened| {
                        instruction_trust::is_trusted(db, manuscript, opened)
                            .map_err(|_| InstructionReadError::Unavailable)?
                            .then_some(())
                            .ok_or(InstructionReadError::Unauthorized)
                    })
                }
                .map_err(|_| SteeringError::Conflict)?;
                input.skill_instructions.extend(prompts);
            }
            _ => {}
        }
    }
    // Bound all newly injected versions across the turn, not merely each
    // individual Steer. Duplicate references retain their identities but do
    // not consume the model instruction budget again.
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0usize;
    let mut add = |prompts: &[research_agent_core::skills::instructions::SkillInstructions]| {
        for prompt in prompts {
            if seen.insert(prompt.identity.clone()) {
                bytes = bytes
                    .checked_add(prompt.contents.len())
                    .ok_or(SteeringError::Capacity)?;
                if bytes > 1_048_576 || seen.len() > 256 {
                    return Err(SteeringError::Capacity);
                }
            }
        }
        Ok(())
    };
    add(&record.input.skill_instructions)?;
    let mut statement = db.prepare("select r.skill_instructions_json from agent_steering_resources r join agent_steering_inputs i on i.input_id=r.input_id where i.turn_id=?1 order by i.ordinal").map_err(|_| SteeringError::Unavailable)?;
    let rows = statement
        .query_map([input.scope.turn_id.to_string()], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|_| SteeringError::Unavailable)?;
    for row in rows {
        let json = row.map_err(|_| SteeringError::Unavailable)?;
        let prompts: Vec<research_agent_core::skills::instructions::SkillInstructions> =
            serde_json::from_str(&json).map_err(|_| SteeringError::Invalid)?;
        add(&prompts)?;
    }
    add(&input.skill_instructions)?;
    input.validate()
}
