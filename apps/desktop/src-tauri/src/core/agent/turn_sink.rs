//! Connects the core runner to the application-owned durable session service.
use super::service::AgentSessionService;
use research_agent_core::{
    conversation::ConversationId,
    protocol::EventPayload,
    runner::{RunError, RunFuture, TurnEventSink},
};
use uuid::Uuid;

pub struct ServiceTurnSink<'a> {
    pub service: &'a AgentSessionService,
    pub conversation_id: ConversationId,
    pub request_id: Uuid,
}

impl TurnEventSink for ServiceTurnSink<'_> {
    fn emit(&self, payload: EventPayload) -> RunFuture<'_, ()> {
        Box::pin(async move {
            self.service
                .commit_payload(self.conversation_id, self.request_id, payload)
                .await
                .map_err(|_| RunError::EventSink)
        })
    }
}
