//! Durable structured input and admission-time skill bodies; no source re-read in history.
use super::*;
use research_agent_core::{skills::instructions::SkillInstructions, user_input::UserInput};

pub(crate) fn read(
    db: &Connection,
    id: Uuid,
) -> Result<(Vec<UserInput>, Vec<SkillInstructions>), SteeringError> {
    let value: Option<(String, String)> = db.query_row(
        "select input_json,skill_instructions_json from agent_steering_resources where input_id=?1",
        [id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(db_error)?;
    match value {
        None => Ok((vec![], vec![])),
        Some((input, instructions)) => Ok((
            serde_json::from_str(&input).map_err(|_| SteeringError::Invalid)?,
            serde_json::from_str(&instructions).map_err(|_| SteeringError::Invalid)?,
        )),
    }
}
pub(super) fn insert(db: &Connection, input: &SteeringInput) -> Result<(), SteeringError> {
    if input.user_input.is_empty() {
        return Ok(());
    }
    db.execute("insert into agent_steering_resources(input_id,input_json,skill_instructions_json) values(?1,?2,?3)", params![input.input_id.to_string(),
        serde_json::to_string(&input.user_input).map_err(|_| SteeringError::Invalid)?,
        serde_json::to_string(&input.skill_instructions).map_err(|_| SteeringError::Invalid)?]).map_err(db_error)?;
    Ok(())
}
pub(super) fn input(
    db: &Connection,
    scope: SteeringScope,
    id: &str,
    text: String,
) -> Result<SteeringInput, SteeringError> {
    let input_id = Uuid::parse_str(id).map_err(|_| SteeringError::Invalid)?;
    let (user_input, skill_instructions) = read(db, input_id)?;
    Ok(SteeringInput {
        scope,
        input_id,
        text,
        user_input,
        skill_instructions,
    })
}
