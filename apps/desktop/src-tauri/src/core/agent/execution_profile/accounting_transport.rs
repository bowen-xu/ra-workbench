//! Frozen native accounting policy, separate from wire conversion and credentials.
use research_agent_core::{
    context::{accounting::ContextAccounting, compaction_strategy::CompactionStrategy},
    model::*,
};
use std::sync::Arc;

pub(super) struct AccountingTransport {
    pub token_budget: Option<Arc<research_agent_core::context::token_budget::TokenBudgetPolicy>>,
    pub history: Option<super::model_transition::ModelHistory>,
    pub policy: super::super::repository::context_policy::SqliteContextPolicy,
    pub inner: Arc<dyn ModelTransport>,
    pub accounting: Arc<ContextAccounting>,
    pub strategy: Arc<dyn CompactionStrategy>,
}
impl AccountingTransport {
    pub fn new(
        policy: super::super::repository::context_policy::SqliteContextPolicy,
        history: Option<super::model_transition::ModelHistory>,
        inner: Arc<dyn ModelTransport>,
        accounting: Arc<ContextAccounting>,
        strategy: Arc<dyn CompactionStrategy>,
        token_budget: Option<Arc<research_agent_core::context::token_budget::TokenBudgetPolicy>>,
    ) -> Self {
        Self {
            history,
            token_budget,
            policy,
            inner,
            strategy,
            accounting,
        }
    }
}
impl ModelTransport for AccountingTransport {
    fn tool_output_policy(&self) -> Option<tool_output::ToolOutputPolicy> {
        self.policy.tool_output_policy()
    }
    fn pre_turn_compaction<'a>(
        &'a self,
        turn: uuid::Uuid,
        model: &'a research_agent_core::protocol::ModelIdentity,
        active: Option<u64>,
    ) -> ModelFuture<'a, Option<Arc<dyn CompactionStrategy>>> {
        Box::pin(async move {
            if self.strategy.binding().kind
                == research_agent_core::context::compaction_strategy::CompactionKind::ResetWindow
            {
                // Reset never needs an old-model inference or credential factory.
                return Ok(None);
            }
            match &self.history {
                Some(history) => history.select(&self.policy, turn, model, active).await,
                None => Ok(None),
            }
        })
    }
    fn prepare_context_policy(&self) -> ModelFuture<'_, ()> {
        self.policy.prepare()
    }
    fn authorize_request<'a>(&'a self, request: &'a ModelRequest) -> ModelFuture<'a, ()> {
        self.inner.authorize_request(request)
    }
    fn prepared_compaction_strategy(&self) -> Option<Arc<dyn CompactionStrategy>> {
        Some(self.strategy.clone())
    }
    fn supports_compaction_kind(
        &self,
        kind: research_agent_core::context::compaction_strategy::CompactionKind,
    ) -> bool {
        self.inner.supports_compaction_kind(kind)
    }
    fn validate_compaction_window<'a>(
        &'a self,
        checkpoint: &'a research_agent_core::context::checkpoint::ContextCheckpoint,
        request: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        Box::pin(async move {
            if request.model != self.accounting.model
                || !self.strategy.accepts_binding(&checkpoint.binding)
            {
                return Err(ModelError::UnsupportedCapability);
            }
            // A consuming view changes no durable producer metadata or state.
            let mut consuming = checkpoint.clone();
            consuming.binding.model = request.model.clone();
            self.inner
                .validate_compaction_window(&consuming, request)
                .await
        })
    }
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn supports_namespace_tools(&self) -> bool {
        self.inner.supports_namespace_tools()
    }
    fn token_budget_policy(
        &self,
    ) -> Option<Arc<research_agent_core::context::token_budget::TokenBudgetPolicy>> {
        self.token_budget.clone()
    }
    fn context_accounting(&self) -> Option<Arc<ContextAccounting>> {
        Some(self.accounting.clone())
    }
    fn start_text_compaction(
        &self,
        request: ModelRequest,
    ) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start_text_compaction(request)
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}
