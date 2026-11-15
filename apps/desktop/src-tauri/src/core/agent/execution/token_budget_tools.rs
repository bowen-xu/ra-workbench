//! Native exact-turn adapter for the migrated TokenBudget tools. No tool-owned
//! compaction or model execution. Session state survives an ended native turn.
use super::*;
use research_agent_core::context::compaction_strategy::CompactionKind;
use research_agent_core::context::token_budget::{
    TokenBudgetPolicy, tools::TokenBudgetToolRuntime,
};
use std::sync::Mutex;

pub(super) struct NativeTokenBudgetTools<'a> {
    execution: &'a AgentExecution,
    service: &'a AgentSessionService,
    record: &'a SubmissionRecord,
    signal: &'a AgentCancellation,
    policy: Arc<TokenBudgetPolicy>,
    request: Mutex<Option<ModelRequest>>,
}
impl<'a> NativeTokenBudgetTools<'a> {
    pub fn new(
        execution: &'a AgentExecution,
        service: &'a AgentSessionService,
        record: &'a SubmissionRecord,
        signal: &'a AgentCancellation,
    ) -> Option<Self> {
        execution
            .transport
            .token_budget_policy()
            .map(|policy| Self {
                execution,
                service,
                record,
                signal,
                policy,
                request: Mutex::new(None),
            })
    }
    pub fn observe(&self, turn: Uuid, request: &ModelRequest) -> Result<(), RunError> {
        if turn != self.record.turn_id
            || request.model != self.record.input.model
            || request.model != self.execution.model
        {
            return Err(ModelError::InvalidRequest.into());
        }
        *self.request.lock().map_err(|_| RunError::EventSink)? = Some(request.clone());
        Ok(())
    }
    async fn authorize(&self) -> Result<(), RunError> {
        if self.signal.is_cancelled() {
            return Err(ModelError::Canceled.into());
        }
        let reset_strategy = self
            .execution
            .compaction_strategy
            .as_ref()
            .is_some_and(|strategy| {
                let binding = strategy.binding();
                binding.model == self.execution.model && binding.kind == CompactionKind::ResetWindow
            });
        if self.record.input.model != self.execution.model || !reset_strategy {
            return Err(ModelError::UnsupportedCapability.into());
        }
        let history = self
            .service
            .open(ConversationId::from_uuid(self.record.conversation_id))
            .await
            .map_err(|_| RunError::EventSink)?;
        if history.active_turn() != Some(self.record.turn_id) {
            return Err(ModelError::InvalidRequest.into());
        }
        Ok(())
    }
}
impl TokenBudgetToolRuntime for NativeTokenBudgetTools<'_> {
    fn request_new_window(&self) -> RunFuture<'_, ()> {
        Box::pin(async move {
            self.authorize().await?;
            self.service
                .new_context_request(self.record.conversation_id, self.record.turn_id, false)
                .await
                .map_err(|_| RunError::EventSink)?;
            Ok(())
        })
    }
    fn tokens_remaining(&self) -> RunFuture<'_, Option<u64>> {
        Box::pin(async move {
            self.authorize().await?;
            let request = self
                .request
                .lock()
                .map_err(|_| RunError::EventSink)?
                .clone()
                .ok_or(ModelError::InvalidRequest)?;
            let checkpoint = self
                .service
                .active_context_checkpoint(self.record.conversation_id)
                .await
                .map_err(|_| RunError::EventSink)?;
            let checkpoint = self.execution.consuming_checkpoint(checkpoint)?;
            let accounting = self
                .execution
                .transport
                .context_accounting()
                .ok_or(ModelError::UnsupportedCapability)?;
            Ok(self
                .policy
                .status(
                    &accounting,
                    &request.model,
                    &request.messages,
                    &request.tools,
                    checkpoint.as_ref(),
                )?
                .base_window_tokens_remaining)
        })
    }
}
