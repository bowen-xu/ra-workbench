//! Serialize queue consumption with all other conversation event writes.
use super::*;
use research_agent_core::{
    model::{ContentPart, ModelMessage},
    protocol::{EventPayload, ItemKind},
    runner::{RunError, RunFuture, TurnInputSource},
    steering::{SteeringScope, SteeringStore},
};

pub struct SessionTurnInput<'a> {
    pub service: &'a AgentSessionService,
    pub store: &'a dyn SteeringStore,
    pub scope: SteeringScope,
    pub evidence_store: Option<&'a dyn research_agent_core::submission::SubmissionStore>,
    pub injected:
        std::sync::Mutex<std::collections::HashSet<research_agent_core::skills::SkillIdentity>>,
}

impl TurnInputSource for SessionTurnInput<'_> {
    fn seal_for_stop(&self, turn_id: Uuid) -> RunFuture<'_, ()> {
        Box::pin(async move {
            if turn_id != self.scope.turn_id {
                return Err(RunError::EventSink);
            }
            let id = ConversationId::from_uuid(self.scope.conversation_id);
            let slot = self
                .service
                .slot(id, false)
                .map_err(|_| RunError::EventSink)?;
            let owner = slot.lock().await;
            let session = owner.as_ref().ok_or(RunError::EventSink)?;
            if session.needs_recovery() || session.projection().active_turn() != Some(turn_id) {
                return Err(RunError::EventSink);
            }
            self.store
                .seal_for_stop(self.scope)
                .await
                .map_err(|_| RunError::EventSink)
        })
    }
    fn has_pending(&self, turn_id: Uuid) -> RunFuture<'_, bool> {
        Box::pin(async move {
            if turn_id != self.scope.turn_id {
                return Err(RunError::EventSink);
            }
            let id = ConversationId::from_uuid(self.scope.conversation_id);
            let slot = self
                .service
                .slot(id, false)
                .map_err(|_| RunError::EventSink)?;
            let owner = slot.lock().await;
            let session = owner.as_ref().ok_or(RunError::EventSink)?;
            if session.needs_recovery() || session.projection().active_turn() != Some(turn_id) {
                return Err(RunError::EventSink);
            }
            self.store
                .pending(self.scope, false)
                .await
                .map(|inputs| !inputs.is_empty())
                .map_err(|_| RunError::EventSink)
        })
    }
    fn has_pending_or_seal(&self, turn_id: Uuid) -> RunFuture<'_, bool> {
        Box::pin(async move {
            if turn_id != self.scope.turn_id {
                return Err(RunError::EventSink);
            }
            let id = ConversationId::from_uuid(self.scope.conversation_id);
            let slot = self
                .service
                .slot(id, false)
                .map_err(|_| RunError::EventSink)?;
            let owner = slot.lock().await;
            let session = owner.as_ref().ok_or(RunError::EventSink)?;
            if session.needs_recovery() || session.projection().active_turn() != Some(turn_id) {
                return Err(RunError::EventSink);
            }
            // Existing transactional store seals only if empty, without changing
            // history. Keep the same session owner as admission and consumption.
            self.store
                .pending(self.scope, true)
                .await
                .map(|inputs| !inputs.is_empty())
                .map_err(|_| RunError::EventSink)
        })
    }
    fn drain(&self, turn_id: Uuid, finish_if_empty: bool) -> RunFuture<'_, Vec<ModelMessage>> {
        Box::pin(async move {
            if turn_id != self.scope.turn_id {
                return Err(RunError::EventSink);
            }
            self.consume(finish_if_empty)
                .await
                .map_err(|_| RunError::EventSink)
        })
    }
}

impl SessionTurnInput<'_> {
    async fn consume(&self, finish_if_empty: bool) -> Result<Vec<ModelMessage>, AgentServiceError> {
        let id = ConversationId::from_uuid(self.scope.conversation_id);
        let slot = self.service.slot(id, false)?;
        let mut owner = slot.lock().await;
        let session = owner.as_mut().ok_or(AgentServiceError::NotOpen)?;
        if session.needs_recovery() {
            return Err(SessionError::RecoveryRequired.into());
        }
        if session.projection().active_turn() != Some(self.scope.turn_id) {
            return Err(AgentServiceError::Active);
        }
        // Hold the session owner through pending lookup AND journal commit. A
        // second consumer cannot insert the same user item from an earlier read.
        let inputs = self
            .store
            .pending(self.scope, finish_if_empty)
            .await
            .map_err(|_| AgentServiceError::Unavailable)?;
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        if inputs.len() > 64 {
            return Err(SessionError::InvalidBatch.into());
        }
        let mut seq = session.projection().seq();
        let timestamp = now_ms()?;
        let mut events = Vec::with_capacity(inputs.len() * 3);
        let mut messages = Vec::with_capacity(inputs.len());
        let mut injected = self
            .injected
            .lock()
            .map_err(|_| AgentServiceError::Unavailable)?
            .clone();
        for input in inputs {
            input.validate().map_err(|_| SessionError::InvalidBatch)?;
            if input.scope != self.scope {
                return Err(SessionError::InvalidBatch.into());
            }
            let item_id = input.input_id;
            let turn_id = self.scope.turn_id;
            for payload in [
                EventPayload::ItemStarted {
                    turn_id,
                    item_id,
                    kind: ItemKind::UserMessage,
                },
                EventPayload::TextDelta {
                    turn_id,
                    item_id,
                    text: input.text.clone(),
                },
                EventPayload::ItemCompleted { turn_id, item_id },
            ] {
                seq = seq.checked_add(1).ok_or(SessionError::InvalidBatch)?;
                events.push(
                    AgentEvent::new(id, seq, timestamp, item_id, payload)
                        .map_err(|_| SessionError::InvalidBatch)?,
                );
            }
            let mut parts = Vec::new();
            {
                for prompt in &input.skill_instructions {
                    if injected.insert(prompt.identity.clone()) {
                        parts.push(ContentPart::Text(prompt.render()));
                    }
                }
            }
            let evidence = if let Some(store) = self.evidence_store {
                store.evidence_content(&input.user_input).await
            } else {
                research_agent_core::user_input::evidence::content(&input.user_input, |_| {
                    Err(research_agent_core::submission::SubmissionError::Unavailable)
                })
            }
            .map_err(|_| AgentServiceError::Unavailable)?;
            parts.extend(evidence);
            parts.push(ContentPart::Text(input.text));
            messages.push(ModelMessage::User {
                content: parts,
                source_item_id: Some(item_id),
            });
        }
        let receipt = session.commit(events).await?;
        *self
            .injected
            .lock()
            .map_err(|_| AgentServiceError::Unavailable)? = injected;
        self.service.notify(id, &receipt);
        Ok(messages)
    }
}

#[cfg(test)]
#[path = "steering/tests.rs"]
mod tests;
