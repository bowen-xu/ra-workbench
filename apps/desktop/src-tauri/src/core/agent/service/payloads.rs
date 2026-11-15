//! Commit validated payload batches while the caller holds the session slot.
use super::*;
use research_agent_core::protocol::EventPayload;

impl AgentSessionService {
    pub(super) async fn commit_payloads_owned(
        &self,
        id: ConversationId,
        request_id: Uuid,
        payloads: Vec<EventPayload>,
        session: &mut DurableConversation,
    ) -> Result<CommittedBatch, AgentServiceError> {
        if payloads.is_empty() || payloads.len() > 500 {
            return Err(SessionError::InvalidBatch.into());
        }
        let timestamp = now_ms()?;
        let mut seq = session.projection().seq();
        let events = payloads
            .into_iter()
            .map(|payload| {
                seq = seq.checked_add(1).ok_or(SessionError::InvalidBatch)?;
                AgentEvent::new(id, seq, timestamp, request_id, payload)
                    .map_err(|_| SessionError::InvalidBatch)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let receipt = session.commit(events).await?;
        self.notify(id, &receipt);
        Ok(receipt)
    }
}
