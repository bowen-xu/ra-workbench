//! Freeze replay provenance with the request. Encoding may read only these
//! historical facts, never a newer live projection or unconsumed steer.
use research_agent_core::{
    model::{CapturedHistoryRef, CapturedTurnRef, ModelError},
    protocol::{ConversationProjection, ItemKind},
};

pub(super) fn capture(
    history: &ConversationProjection,
    after_seq: u64,
) -> Result<CapturedHistoryRef, ModelError> {
    // This is provenance, not a model-history reconstruction. A safe mid-turn
    // compaction freezes an active projection too; it must not invent a closed
    // turn just to capture its accepted source identities.
    if after_seq > history.seq() || history.has_unknown_tool_outcome() {
        return Err(ModelError::InvalidRequest);
    }
    let turns = history
        .turn_order()
        .iter()
        .filter_map(|turn_id| {
            let turn = history.turn(*turn_id)?;
            if !turn.task_kind.is_regular() {
                return None;
            }
            Some(CapturedTurnRef {
                turn_id: *turn_id,
                user_item_ids: turn
                    .item_order
                    .iter()
                    .filter(|id| {
                        turn.items.get(id).is_some_and(|item| {
                            matches!(
                                item.kind,
                                ItemKind::UserMessage | ItemKind::HookPrompt { .. }
                            ) && item.completed
                                && !item.interrupted
                        })
                    })
                    .copied()
                    .collect(),
            })
        })
        .collect();
    let captured = CapturedHistoryRef {
        conversation_id: *history.conversation_id().as_uuid(),
        through_seq: history.seq(),
        after_seq,
        turns,
    };
    captured.validate()?;
    Ok(captured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind},
    };
    use uuid::Uuid;

    #[test]
    fn active_compaction_provenance_keeps_the_frozen_watermark_and_user_identity() {
        let mut history = ConversationProjection::new(ConversationId::from_uuid(Uuid::new_v4()));
        let turn = Uuid::new_v4();
        let user = Uuid::new_v4();
        for payload in [
            EventPayload::TurnStarted {
                turn_id: turn,
                context_snapshot_id: Uuid::new_v4(),
                task_kind: TaskKind::Regular,
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
            },
            EventPayload::ItemStarted {
                turn_id: turn,
                item_id: user,
                kind: ItemKind::UserMessage,
            },
            EventPayload::TextDelta {
                turn_id: turn,
                item_id: user,
                text: "input".into(),
            },
            EventPayload::ItemCompleted {
                turn_id: turn,
                item_id: user,
            },
        ] {
            let event = AgentEvent::new(
                history.conversation_id(),
                history.seq() + 1,
                1,
                Uuid::new_v4(),
                payload,
            )
            .unwrap();
            history.apply(&event).unwrap();
        }
        assert_eq!(history.active_turn(), Some(turn));
        let frozen = capture(&history, history.seq()).unwrap();
        assert_eq!((frozen.after_seq, frozen.through_seq), (4, 4));
        assert_eq!(frozen.turns[0].user_item_ids, vec![user]);
        assert!(capture(&history, 5).is_err());
        let hook_item = Uuid::new_v4();
        let hook = AgentEvent::new(
            history.conversation_id(),
            5,
            1,
            Uuid::new_v4(),
            EventPayload::ItemAccepted {
                turn_id: turn,
                item_id: hook_item,
                kind: ItemKind::HookPrompt {
                    invocation_id: Uuid::new_v4(),
                    fragments: vec![research_agent_core::protocol::HookPromptFragment {
                        run_id: Uuid::new_v4(),
                        input_text: "<hook_prompt>retry</hook_prompt>".into(),
                    }],
                },
                text: String::new(),
                phase: None,
                source: None,
            },
        )
        .unwrap();
        history.apply(&hook).unwrap();
        assert_eq!(
            capture(&history, 0).unwrap().turns[0].user_item_ids,
            vec![user, hook_item]
        );
    }
}
