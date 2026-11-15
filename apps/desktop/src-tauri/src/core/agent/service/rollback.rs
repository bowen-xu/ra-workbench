//! History rollback shares task admission and durable session ownership.
use super::*;
use crate::agent::tasks::TaskRegistryError;
use research_agent_core::protocol::EventPayload;

#[derive(Clone, Copy)]
pub struct ConversationRollbackRequest {
    pub conversation_id: ConversationId,
    pub request_id: Uuid,
    pub expected_watermark: u64,
    pub expected_last_turn: Uuid,
    pub user_turns: u32,
}

impl AgentSessionService {
    /// Backend operation only: hosts must authorize their binding before calling.
    /// The owned worker retains admission and the session across waiter cancellation.
    pub async fn rollback_conversation(
        self: &Arc<Self>,
        request: ConversationRollbackRequest,
    ) -> Result<CommittedBatch, AgentServiceError> {
        if request.request_id.is_nil()
            || request.expected_last_turn.is_nil()
            || request.user_turns == 0
            || request.expected_watermark >= 9_007_199_254_740_991
        {
            return Err(SessionError::InvalidBatch.into());
        }
        let service = self.clone();
        tokio::spawn(async move { service.rollback_owned(request).await })
            .await
            .map_err(|_| AgentServiceError::Unavailable)?
    }

    async fn rollback_owned(
        &self,
        request: ConversationRollbackRequest,
    ) -> Result<CommittedBatch, AgentServiceError> {
        let id = request.conversation_id;
        let _admission = self.tasks.reserve_idle(id).map_err(|error| match error {
            TaskRegistryError::Active | TaskRegistryError::Capacity => AgentServiceError::Active,
            _ => AgentServiceError::Unavailable,
        })?;
        let slot = self.slot(id, true)?;
        let mut owner = slot.lock().await;
        if owner.is_none() {
            *owner = Some(DurableConversation::resume(self.store.clone(), fresh_lease(id)).await?);
        }
        let session = owner.as_mut().ok_or(AgentServiceError::NotOpen)?;
        if session.needs_recovery() || session.projection().has_unknown_tool_outcome() {
            return Err(SessionError::RecoveryRequired.into());
        }
        if session.projection().active_turn().is_some() {
            return Err(AgentServiceError::Active);
        }
        let payload = EventPayload::ConversationRolledBack {
            turn_id: request.expected_last_turn,
            user_turns: request.user_turns,
        };
        if session.projection().seq() != request.expected_watermark {
            if session.projection().seq() > request.expected_watermark {
                let page = self
                    .store
                    .history(HistoryQuery::new(id, request.expected_watermark, 1)?)
                    .await?;
                if let Some(event) = page.items.first()
                    && event.request_id() == request.request_id
                    && event.payload() == &payload
                {
                    return Ok(CommittedBatch {
                        watermark: event.seq(),
                        events: vec![event.clone()],
                    });
                }
            }
            return Err(AgentServiceError::RollbackConflict);
        }
        let event = AgentEvent::new(
            id,
            request.expected_watermark + 1,
            now_ms()?,
            request.request_id,
            payload,
        )
        .map_err(|_| SessionError::InvalidBatch)?;
        let receipt = session.commit(vec![event]).await?;
        self.notify(id, &receipt);
        Ok(receipt)
    }
}

#[cfg(test)]
#[path = "rollback/queue_start_tests.rs"]
mod queue_start_tests;
#[cfg(test)]
#[path = "rollback/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "rollback/window_state_tests.rs"]
mod window_state_tests;
