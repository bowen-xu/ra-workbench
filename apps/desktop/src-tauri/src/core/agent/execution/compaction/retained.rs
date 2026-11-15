use super::*;

#[cfg(test)]
#[path = "retained/tests.rs"]
mod tests;

pub(super) fn retained_users(
    history: &ConversationProjection,
    previous: Option<&ContextCheckpoint>,
    budget: usize,
    remote_source: Option<&[ModelMessage]>,
    text_policy: Option<&dyn research_agent_core::context::retained_text::RetainedTextPolicy>,
    user_images: &RetainedUserImages,
) -> Result<Vec<ModelMessage>, RunError> {
    let mut users = Vec::new();
    if let Some(previous) = previous {
        if let Some(source) = remote_source {
            use sha2::{Digest, Sha256};
            let images: HashMap<_, _> = source
                .iter()
                .flat_map(|message| match message {
                    ModelMessage::User { content: parts, .. }
                    | ModelMessage::Assistant { content: parts, .. } => parts.as_slice(),
                    _ => &[],
                })
                .filter_map(|part| match part {
                    ContentPart::Image { bytes, .. } => {
                        Some((format!("{:x}", Sha256::digest(bytes)), bytes.clone()))
                    }
                    _ => None,
                })
                .collect();
            let retained: Vec<_> = previous
                .retained
                .iter()
                .filter(|message| matches!(message, StoredMessage::User { .. }))
                .cloned()
                .collect();
            users.extend(
                research_agent_core::context::checkpoint::restore_messages(&retained, &|hash| {
                    images.get(hash).cloned()
                })
                .map_err(compaction_error)?,
            );
            // Older checkpoints stored images in synthetic canonical User
            // messages. Preserve those typed payloads at their explicit slot;
            // do not infer which retained text they belong to or retain rules.
            if let Some(context) = &previous.canonical_context {
                let image_messages: Vec<_> = context
                    .messages
                    .iter()
                    .filter_map(|message| {
                        let StoredMessage::User {
                            content,
                            source_item_id,
                        } = message
                        else {
                            return None;
                        };
                        let content: Vec<_> = content
                            .iter()
                            .filter(|part| matches!(part, StoredContent::Image { .. }))
                            .cloned()
                            .collect();
                        (!content.is_empty()).then_some(StoredMessage::User {
                            source_item_id: *source_item_id,
                            content,
                        })
                    })
                    .collect();
                let restored = research_agent_core::context::checkpoint::restore_messages(
                    &image_messages,
                    &|hash| images.get(hash).cloned(),
                )
                .map_err(compaction_error)?;
                let position = previous
                    .retained
                    .iter()
                    .take(context.before_retained)
                    .filter(|message| matches!(message, StoredMessage::User { .. }))
                    .count();
                users.splice(position..position, restored);
            }
        } else {
            for message in &previous.retained {
                if let StoredMessage::User {
                    content,
                    source_item_id,
                } = message
                {
                    let parts = content
                        .iter()
                        .filter_map(|part| match part {
                            StoredContent::Text { text } => Some(ContentPart::Text(text.clone())),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    if !parts.is_empty() {
                        users.push(ModelMessage::User {
                            content: parts,
                            source_item_id: *source_item_id,
                        });
                    }
                }
            }
        }
    }
    let tail = previous
        .map(|window| history.context_tail(window.source_watermark))
        .transpose()
        .map_err(|_| ModelError::InvalidRequest)?;
    let view = tail.as_ref().unwrap_or(history);
    for turn_id in view.turn_order() {
        if let Some(turn) = view.turn(*turn_id) {
            for item_id in &turn.item_order {
                if let Some(item) = turn.items.get(item_id)
                    && item.completed
                    && matches!(
                        item.kind,
                        research_agent_core::protocol::ItemKind::UserMessage
                    )
                {
                    let mut parts = Vec::new();
                    if remote_source.is_some()
                        && user_images.contains_key(turn_id)
                        && *item_id == original_user_item(history, *turn_id)?
                    {
                        parts.extend(user_images[turn_id].clone());
                    }
                    if !item.text.is_empty() {
                        parts.push(ContentPart::Text(item.text.clone()));
                    }
                    if !parts.is_empty() {
                        users.push(ModelMessage::User {
                            content: parts,
                            source_item_id: Some(*item_id),
                        });
                    } else if remote_source.is_some() {
                        return Err(ModelError::InvalidRequest.into());
                    }
                }
            }
        }
    }
    let users = if let Some(policy) = text_policy {
        research_agent_core::context::retained_text::retain_users_with_byte_budget(
            users,
            policy,
            budget,
            message_bytes,
        )
        .map_err(compaction_error)?
    } else {
        if remote_source.is_some()
            && users.last().is_some_and(|user| {
                message_bytes(std::slice::from_ref(user)).is_none_or(|size| size > budget)
            })
        {
            return Err(RunError::Budget);
        }
        users
    };
    let mut remaining = budget;
    let mut kept = Vec::new();
    for user in users.into_iter().rev() {
        let Some(next) =
            message_bytes(std::slice::from_ref(&user)).and_then(|size| remaining.checked_sub(size))
        else {
            break;
        };
        remaining = next;
        kept.push(user);
    }
    kept.reverse();
    Ok(kept)
}
