//! Restore only this turn's frozen explicit prompts and successful durable
//! skill loads. Never read a changed file or interpret arbitrary model prose as
//! host context. Reference/script tool output remains historical evidence.
use super::*;
use research_agent_core::{
    protocol::ItemKind,
    skills::{SkillIdentity, instructions::SkillInstructions},
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Loaded {
    skill: Body,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Body {
    identity: SkillIdentity,
    name: String,
    body: String,
    execution_directory: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoadArgs {
    identity: SkillIdentity,
}

pub(super) async fn restore(
    submissions: &dyn SubmissionStore,
    record: &SubmissionRecord,
    history: &ConversationProjection,
) -> Result<Vec<ContentPart>, RunError> {
    let turn = history.turn(record.turn_id).ok_or(RunError::EventSink)?;
    let mut known: HashMap<_, _> = record
        .input
        .skill_instructions
        .iter()
        .map(|prompt| (prompt.identity.clone(), prompt.clone()))
        .collect();
    let mut extra = Vec::new();
    for input in submissions
        .get_history_steering(record.conversation_id, record.turn_id)
        .await
        .map_err(|_| RunError::EventSink)?
    {
        if turn.items.get(&input.input_id).is_none_or(|item| {
            !item.completed
                || !matches!(item.kind, ItemKind::UserMessage)
                || item.text != input.text
        }) {
            continue;
        }
        for prompt in input.skill_instructions {
            insert(&mut known, &mut extra, prompt)?;
        }
    }
    for id in &turn.item_order {
        let result = turn.items.get(id).ok_or(RunError::EventSink)?;
        let ItemKind::ToolResult {
            call_id,
            is_error: Some(false),
        } = &result.kind
        else {
            continue;
        };
        if !result.completed {
            continue;
        }
        let Some(call) = turn.items.values().find(|item| item.completed && matches!(&item.kind,
            ItemKind::ToolCall { call_id: id, name, .. } if id == call_id && name == "research_load_skill")) else { continue; };
        let args: LoadArgs =
            serde_json::from_str(&call.text).map_err(|_| ModelError::InvalidStream)?;
        let loaded: Loaded =
            serde_json::from_str(&result.text).map_err(|_| ModelError::InvalidStream)?;
        let body = loaded.skill;
        let descriptor = record
            .input
            .skills
            .iter()
            .find(|skill| skill.descriptor.identity == args.identity)
            .ok_or(ModelError::InvalidStream)?;
        if body.identity != args.identity
            || body.name != descriptor.descriptor.name
            || body.body.is_empty()
            || body.body.len() > 1024 * 1024
            || body.execution_directory != crate::agent::skills::packages::relative(&body.identity)
        {
            return Err(ModelError::InvalidStream.into());
        }
        let contents = crate::agent::skills::injection::frozen_contents(&body.body, &body.identity);
        insert(
            &mut known,
            &mut extra,
            SkillInstructions {
                identity: body.identity,
                name: body.name,
                contents,
            },
        )?;
    }
    Ok(extra)
}
fn insert(
    known: &mut HashMap<SkillIdentity, SkillInstructions>,
    out: &mut Vec<ContentPart>,
    prompt: SkillInstructions,
) -> Result<(), RunError> {
    if let Some(previous) = known.get(&prompt.identity) {
        if previous != &prompt {
            return Err(ModelError::InvalidStream.into());
        }
    } else {
        out.push(ContentPart::Text(prompt.render()));
        known.insert(prompt.identity.clone(), prompt);
    }
    Ok(())
}
