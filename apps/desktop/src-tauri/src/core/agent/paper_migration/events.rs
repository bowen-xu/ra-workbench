//! Stable legacy identities and lossless message-to-event conversion.
use super::LegacyTurn;
use research_agent_core::protocol::{
    EventPayload, ItemKind, ModelIdentity, TurnOutcome, TurnPhase,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct LegacyMessage {
    role: String,
    content: String,
}

pub(super) fn decode_messages(turn: &LegacyTurn) -> Vec<(String, String)> {
    let parsed = serde_json::from_str::<Vec<LegacyMessage>>(&turn.messages_json)
        .ok()
        .filter(|messages| {
            messages
                .iter()
                .all(|m| matches!(m.role.as_str(), "user" | "assistant"))
        })
        .map(|messages| {
            messages
                .into_iter()
                .map(|m| (m.role, m.content))
                .filter(|(_, c)| !c.is_empty())
                .collect()
        })
        .filter(|messages: &Vec<_>| !messages.is_empty());
    match parsed {
        Some(mut messages) => {
            // Some old turns persisted only the structured user message while
            // keeping the assistant text in the denormalized column. Preserve
            // that field when the structured payload is incomplete.
            if !turn.user_message.is_empty() && !messages.iter().any(|(role, _)| role == "user") {
                messages.insert(0, ("user".into(), turn.user_message.clone()));
            }
            if !turn.assistant_message.is_empty()
                && !messages.iter().any(|(role, _)| role == "assistant")
            {
                messages.push(("assistant".into(), turn.assistant_message.clone()));
            }
            messages
        }
        None => {
            let mut messages = Vec::new();
            if !turn.user_message.is_empty() {
                messages.push(("user".into(), turn.user_message.clone()));
            }
            if !turn.assistant_message.is_empty() {
                messages.push(("assistant".into(), turn.assistant_message.clone()));
            }
            messages
        }
    }
}

fn deterministic_uuid(label: &str) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&Sha256::digest(label.as_bytes())[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

pub(super) fn legacy_context_snapshot_id(session_id: &str, turn_id: &str) -> Uuid {
    deterministic_uuid(&format!(
        "research-assistant:legacy-paper-context:{session_id}:{turn_id}"
    ))
}

pub(super) fn legacy_turn_id(session_id: &str, turn_id: &str) -> Uuid {
    Uuid::parse_str(turn_id).unwrap_or_else(|_| {
        deterministic_uuid(&format!(
            "research-assistant:legacy-paper-turn:{session_id}:{turn_id}"
        ))
    })
}

pub(super) fn legacy_request_id(session_id: &str, turn_id: &str) -> Uuid {
    deterministic_uuid(&format!(
        "research-assistant:legacy-paper-request:{session_id}:{turn_id}"
    ))
}

pub(super) fn turn_payloads(session_id: &str, turn: &LegacyTurn) -> Vec<EventPayload> {
    let turn_id = legacy_turn_id(session_id, &turn.id);
    let model = ModelIdentity {
        provider_id: if turn.provider_id.trim().is_empty() {
            "legacy".into()
        } else {
            turn.provider_id.clone()
        },
        model_id: if turn.model_id.trim().is_empty() {
            "legacy".into()
        } else {
            turn.model_id.clone()
        },
        revision: None,
    };
    let attempt_id = deterministic_uuid(&format!(
        "research-assistant:legacy-paper-attempt:{session_id}:{}",
        turn.id
    ));
    let mut events = vec![
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id,
            context_snapshot_id: legacy_context_snapshot_id(session_id, &turn.id),
            model: model.clone(),
        },
        EventPayload::PhaseChanged {
            turn_id,
            phase: TurnPhase::RunningModel,
        },
        EventPayload::ModelRequestStarted {
            turn_id,
            attempt_id,
            model: model.clone(),
        },
        EventPayload::ModelResponseStarted {
            turn_id,
            attempt_id,
            response_id: format!("legacy-{turn_id}"),
            model: Some(model),
        },
    ];
    for (ordinal, (role, content)) in decode_messages(turn).into_iter().enumerate() {
        let item_id = deterministic_uuid(&format!(
            "research-assistant:legacy-paper-item:{session_id}:{}:{ordinal}",
            turn.id
        ));
        events.push(EventPayload::ItemStarted {
            turn_id,
            item_id,
            kind: if role == "user" {
                ItemKind::UserMessage
            } else {
                ItemKind::AssistantMessage
            },
        });
        let mut start = 0;
        for (count, (index, _)) in content.char_indices().enumerate() {
            if count > 0 && count % 65_536 == 0 {
                events.push(EventPayload::TextDelta {
                    turn_id,
                    item_id,
                    text: content[start..index].to_owned(),
                });
                start = index;
            }
        }
        if start < content.len() {
            events.push(EventPayload::TextDelta {
                turn_id,
                item_id,
                text: content[start..].to_owned(),
            });
        }
        events.push(EventPayload::ItemCompleted { turn_id, item_id });
    }
    events.push(EventPayload::ModelRequestFinished {
        turn_id,
        attempt_id,
        input_tokens: turn.input_tokens.and_then(|v| u64::try_from(v).ok()),
        output_tokens: turn.output_tokens.and_then(|v| u64::try_from(v).ok()),
    });
    if turn.input_tokens.is_some() || turn.output_tokens.is_some() {
        events.push(EventPayload::Usage {
            turn_id,
            input_tokens: turn.input_tokens.and_then(|v| u64::try_from(v).ok()),
            output_tokens: turn.output_tokens.and_then(|v| u64::try_from(v).ok()),
        });
    }
    let outcome = match turn.status.as_str() {
        "succeeded" => TurnOutcome::Completed,
        "canceled" => TurnOutcome::Canceled,
        "interrupted" => TurnOutcome::Interrupted,
        _ => TurnOutcome::Failed,
    };
    events.push(EventPayload::TurnFinished { turn_id, outcome });
    events
}
