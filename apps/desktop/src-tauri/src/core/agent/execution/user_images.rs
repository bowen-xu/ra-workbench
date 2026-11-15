//! Associate frozen submission images with the original admission item, never
//! with an equal-text steer or a synthetic rule/context message.
use super::*;

/// Keep captured image parts available to checkpoint replay, without retaining
/// empty image slots for text-only selections.
pub(super) async fn evidence_parts(
    input: &[research_agent_core::user_input::UserInput],
    turn_id: Uuid,
    retained: &mut RetainedUserImages,
    submissions: &dyn SubmissionStore,
) -> Result<Vec<ContentPart>, RunError> {
    let parts = submissions
        .evidence_content(input)
        .await
        .map_err(image_error)?;
    let images: Vec<_> = parts
        .iter()
        .filter(|part| matches!(part, ContentPart::Image { .. }))
        .cloned()
        .collect();
    if !images.is_empty() {
        retained.entry(turn_id).or_default().extend(images);
    }
    Ok(parts)
}

pub(super) fn original_user_item(
    history: &ConversationProjection,
    turn_id: Uuid,
) -> Result<Uuid, RunError> {
    let turn = history.turn(turn_id).ok_or(ModelError::InvalidRequest)?;
    if !turn.task_kind.is_regular() {
        return Err(ModelError::InvalidRequest.into());
    }
    let item = turn
        .item_order
        .iter()
        .find_map(|id| {
            turn.items
                .get(id)
                .filter(|item| {
                    matches!(
                        item.kind,
                        research_agent_core::protocol::ItemKind::UserMessage
                    )
                })
                .map(|item| (*id, item))
        })
        .ok_or(ModelError::InvalidRequest)?;
    if !item.1.completed {
        return Err(ModelError::InvalidRequest.into());
    }
    Ok(item.0)
}

pub(super) fn image_parts(
    images: &[research_agent_core::submission::SubmissionImage],
) -> Result<Vec<ContentPart>, RunError> {
    Ok(
        research_agent_core::submission::build_user_content("", images)
            .map_err(image_error)?
            .into_iter()
            .filter(|part| matches!(part, ContentPart::Image { .. }))
            .collect(),
    )
}
