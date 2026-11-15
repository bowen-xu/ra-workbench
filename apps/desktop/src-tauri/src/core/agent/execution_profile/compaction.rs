//! Explicit native provider/model policy composed with the actual transport.
//! Codex task strategy routing: e363b08c9175ac1cbe5893615dd2cb9ddf95043b,
//! core/src/tasks/compact.rs, Apache-2.0. No URL capability inference.
use super::*;
use research_agent_core::context::{
    compaction_strategy::{CompactionBinding, CompactionKind, CompactionStrategy},
    text_compaction::TextCompactionStrategy,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeCompactionPolicy {
    TokenBudgetReset,
    Text,
    ChatGptResponsesV2,
    ChatGptResponsesFullWindow,
}
impl NativeCompactionPolicy {
    pub(crate) fn binding(self, model: ModelIdentity) -> CompactionBinding {
        let (strategy_id, kind) = match self {
            Self::TokenBudgetReset => ("token-budget.reset-window", CompactionKind::ResetWindow),
            Self::Text => ("research.text.v1", CompactionKind::TextSummary),
            Self::ChatGptResponsesV2 => (
                "research.openai.responses-v2.v1",
                CompactionKind::RemoteOpaque,
            ),
            Self::ChatGptResponsesFullWindow => (
                "research.openai.responses-window.v1",
                CompactionKind::RemoteWindow,
            ),
        };
        CompactionBinding {
            model,
            strategy_id: strategy_id.into(),
            state_schema: 1,
            kind,
        }
    }
    pub(crate) fn accepts_remote_binding(
        self,
        target: &CompactionBinding,
        stored: &CompactionBinding,
    ) -> bool {
        matches!(
            self,
            Self::ChatGptResponsesV2 | Self::ChatGptResponsesFullWindow
        ) && target.model.provider_id == "openai"
            && stored.model.provider_id == "openai"
            // Model admission and same-account authorization happen at the
            // transport boundary; stored producer identity remains untouched.
            && *target == self.binding(target.model.clone())
            && [Self::ChatGptResponsesV2, Self::ChatGptResponsesFullWindow]
                .into_iter()
                .any(|producer| *stored == producer.binding(stored.model.clone()))
            && execution_profile_with_compaction(
                &target.model,
                AgentExecutionMode::ChatGptDefault,
                self,
            )
            .is_ok()
            && execution_profile_with_compaction(
                &stored.model,
                AgentExecutionMode::ChatGptDefault,
                self,
            )
            .is_ok()
    }
    pub fn responses_strategy(
        self,
        transport: Arc<super::super::model::responses_transport::ResponsesTransport>,
        model: ModelIdentity,
    ) -> Result<Arc<dyn CompactionStrategy>, AgentModelPreparationError> {
        self.responses_strategy_prepared(transport, model, None)
    }
    pub(crate) fn responses_strategy_with_accounting(
        self,
        transport: Arc<super::super::model::responses_transport::ResponsesTransport>,
        model: ModelIdentity,
        accounting: Arc<research_agent_core::context::accounting::ContextAccounting>,
    ) -> Result<Arc<dyn CompactionStrategy>, AgentModelPreparationError> {
        self.responses_strategy_prepared(transport, model, Some(accounting))
    }
    fn responses_strategy_prepared(
        self,
        transport: Arc<super::super::model::responses_transport::ResponsesTransport>,
        model: ModelIdentity,
        accounting: Option<Arc<research_agent_core::context::accounting::ContextAccounting>>,
    ) -> Result<Arc<dyn CompactionStrategy>, AgentModelPreparationError> {
        match self {
            Self::TokenBudgetReset => reset_strategy(model),
            Self::Text => text_strategy(transport, model),
            Self::ChatGptResponsesV2 => {
                super::super::model::responses_transport::ResponsesV2CompactionStrategy::new(
                    transport,
                    self.binding(model),
                )
                .and_then(|strategy| match accounting {
                    Some(accounting) => strategy.with_input_accounting(accounting),
                    None => Ok(strategy),
                })
                .and_then(|strategy| {
                    strategy.with_retained_text_policy(Arc::new(
                        research_agent_core::context::retained_text::Utf8RetainedTextPolicy {
                            max_tokens: 64_000,
                        },
                    ))
                })
                .and_then(|strategy| {
                    strategy
                        .with_retry_policy(research_agent_core::resilience::RetryPolicy::DEFAULT)
                        .map_err(Into::into)
                })
                .map(|strategy| Arc::new(strategy) as Arc<dyn CompactionStrategy>)
                .map_err(|_| research_agent_core::model::ModelError::InvalidRequest.into())
            }
            Self::ChatGptResponsesFullWindow => {
                super::super::model::responses_transport::ResponsesLegacyCompactionStrategy::new(
                    transport,
                    self.binding(model),
                )
                .and_then(|strategy| match accounting {
                    Some(accounting) => strategy.with_input_accounting(accounting),
                    None => Ok(strategy),
                })
                .map(|strategy| {
                    Arc::new(strategy.with_retry_policy(
                    super::super::model::responses_transport::LegacyCompactionRetryPolicy::NATIVE,
                )) as Arc<dyn CompactionStrategy>
                })
                .map_err(|_| research_agent_core::model::ModelError::InvalidRequest.into())
            }
        }
    }
}
pub(super) fn reset_strategy(
    model: ModelIdentity,
) -> Result<Arc<dyn CompactionStrategy>, AgentModelPreparationError> {
    research_agent_core::context::token_budget::reset::TokenBudgetResetStrategy::new(model)
        .map(|strategy| Arc::new(strategy) as Arc<dyn CompactionStrategy>)
        .map_err(|_| research_agent_core::model::ModelError::InvalidRequest.into())
}
pub(super) fn text_strategy(
    transport: Arc<dyn ModelTransport>,
    model: ModelIdentity,
) -> Result<Arc<dyn CompactionStrategy>, AgentModelPreparationError> {
    text_strategy_with_prompt(transport, model, None)
}

pub(super) fn text_strategy_with_prompt(
    transport: Arc<dyn ModelTransport>,
    model: ModelIdentity,
    prompt: Option<&str>,
) -> Result<Arc<dyn CompactionStrategy>, AgentModelPreparationError> {
    TextCompactionStrategy::new(transport, NativeCompactionPolicy::Text.binding(model))
        .and_then(|strategy| match prompt {
            Some(prompt) => strategy.with_compact_prompt(prompt.into()),
            None => Ok(strategy),
        })
        .and_then(|strategy| {
            strategy.with_retry_policy(
                research_agent_core::resilience::RetryPolicy::DEFAULT,
                Arc::new(TextRetryWait),
            )
        })
        .map(|strategy| Arc::new(strategy) as Arc<dyn CompactionStrategy>)
        .map_err(|_| research_agent_core::model::ModelError::InvalidRequest.into())
}

// The task owner drops this future on cancellation; no detached timer or runner.
struct TextRetryWait;
impl research_agent_core::runner::RetryWait for TextRetryWait {
    fn wait(&self, delay_ms: u64) -> research_agent_core::runner::RunFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            Ok(())
        })
    }
}

/// Native-only read opt-in; generic Responses strategies remain exactly bound.
pub(super) struct NativeReadStrategy {
    pub inner: Arc<dyn CompactionStrategy>,
    pub policy: NativeCompactionPolicy,
}
impl CompactionStrategy for NativeReadStrategy {
    fn text_compaction_prompt(&self) -> Option<&str> {
        self.inner.text_compaction_prompt()
    }
    fn input_accounting(
        &self,
    ) -> Option<Arc<research_agent_core::context::accounting::ContextAccounting>> {
        self.inner.input_accounting()
    }
    fn binding(&self) -> CompactionBinding {
        self.inner.binding()
    }
    fn execution_model(&self) -> ModelIdentity {
        self.inner.execution_model()
    }
    fn accepts_binding(&self, stored: &CompactionBinding) -> bool {
        self.inner.accepts_binding(stored)
            || self.policy.accepts_remote_binding(&self.binding(), stored)
    }
    fn retention_policy(
        &self,
    ) -> Option<research_agent_core::context::compaction_strategy::CompactionRetention> {
        self.inner.retention_policy()
    }
    fn retained_text_policy(
        &self,
    ) -> Option<Arc<dyn research_agent_core::context::retained_text::RetainedTextPolicy>> {
        self.inner.retained_text_policy()
    }
    fn validate_output(
        &self,
        output: &research_agent_core::context::compaction_strategy::CompactionOutput,
    ) -> Result<(), research_agent_core::context::compaction_strategy::CompactionFailure> {
        self.inner.validate_output(output)
    }
    fn validate_stored_output(
        &self,
        binding: &CompactionBinding,
        output: &research_agent_core::context::compaction_strategy::CompactionOutput,
    ) -> Result<(), research_agent_core::context::compaction_strategy::CompactionFailure> {
        use research_agent_core::context::compaction_strategy::CompactionFailure;
        if !self.accepts_binding(binding) {
            return Err(CompactionFailure::Incompatible);
        }
        if *binding == NativeCompactionPolicy::ChatGptResponsesV2.binding(binding.model.clone()) {
            super::super::model::responses_transport::validate_v2_output(output)
        } else if *binding
            == NativeCompactionPolicy::ChatGptResponsesFullWindow.binding(binding.model.clone())
        {
            super::super::model::responses_transport::validate_legacy_output(output)
        } else {
            Err(CompactionFailure::Incompatible)
        }
    }
    fn compact(
        &self,
        request: research_agent_core::context::compaction_strategy::CompactionRequest,
    ) -> research_agent_core::context::compaction_strategy::CompactionFuture<
        '_,
        research_agent_core::context::compaction_strategy::CompactionCandidate,
    > {
        self.inner.compact(request)
    }
}
