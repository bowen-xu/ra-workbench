//! Native model policy, frozen before transport composition. The renderer cannot
//! choose thresholds; the token estimator and provider window remain separate.
use super::*;
use research_agent_core::context::compaction_strategy::CompactionBudget;

/// Pinned models-manager catalog support; an unknown revision needs its own
/// reviewed provider profile rather than inheriting this undated capability.
pub(super) fn supports_reasoning_summary(model: &ModelIdentity) -> bool {
    model.provider_id == "openai" && model.model_id == "gpt-5.6-terra" && model.revision.is_none()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeContextBudgetPolicy {
    pub scope: research_agent_core::context::accounting::AutoCompactionScope,
    pub effective_percent: u8,
    pub configured_trigger_tokens: Option<u64>,
}
impl NativeContextBudgetPolicy {
    pub const REVIEWED_DEFAULT: Self = Self {
        scope: research_agent_core::context::accounting::AutoCompactionScope::Total,
        effective_percent: 95,
        configured_trigger_tokens: None,
    };
    fn validate(self) -> Result<(), UnsupportedExecutionProfile> {
        if self.effective_percent == 0
            || self.effective_percent > 100
            || self.configured_trigger_tokens == Some(0)
        {
            return Err(UnsupportedExecutionProfile);
        }
        Ok(())
    }
    pub(super) fn resolve(
        self,
        window: Option<u64>,
        reserve: u32,
    ) -> Result<Option<CompactionBudget>, UnsupportedExecutionProfile> {
        self.validate()?;
        window
            .map(|context_window_tokens| {
                let budget = CompactionBudget {
                    context_window_tokens,
                    effective_percent: self.effective_percent,
                    configured_trigger_tokens: self.configured_trigger_tokens,
                    output_reserve_tokens: u64::from(reserve),
                };
                budget
                    .limits_for_scope(self.scope)
                    .map_err(|_| UnsupportedExecutionProfile)?;
                Ok(budget)
            })
            .transpose()
    }
}
impl AgentExecutionProfile {
    /// Explicit capability/configuration injection for the reviewed Responses
    /// model. The pinned catalog defaults to none; this is not renderer input.
    pub fn with_responses_reasoning_summary(
        mut self,
        model: &ModelIdentity,
        summary: Option<super::super::model::responses_reasoning::ResponsesReasoningSummary>,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        if &self.model != model || (summary.is_some() && !supports_reasoning_summary(model)) {
            return Err(UnsupportedExecutionProfile);
        }
        self.reasoning_summary = summary;
        Ok(self)
    }
    /// Trusted host/provider configuration bound to this exact model. File-based
    /// settings must be resolved by the host before admission, never during retry.
    pub fn with_compact_prompt(
        mut self,
        model: &ModelIdentity,
        prompt: Option<String>,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        if &self.model != model
            || prompt.as_ref().is_some_and(|prompt| {
                self.compaction_policy != NativeCompactionPolicy::Text
                    || research_agent_core::context::text_compaction::validate_compact_prompt(
                        prompt,
                    )
                    .is_err()
            })
        {
            return Err(UnsupportedExecutionProfile);
        }
        self.compact_prompt = prompt;
        Ok(self)
    }
    /// Trusted supplier compatibility metadata only; never synthesize a hash from
    /// model names, endpoints or window sizes. None retains unknown compatibility.
    pub fn with_compaction_compatibility_hash(
        mut self,
        model: &ModelIdentity,
        hash: Option<String>,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        if &self.model != model
            || hash.as_ref().is_some_and(|hash| {
                hash.trim().is_empty() || hash.len() > 256 || hash.chars().any(char::is_control)
            })
        {
            return Err(UnsupportedExecutionProfile);
        }
        self.comp_hash = hash;
        Ok(self)
    }
    /// Trusted supplier/model composition only. Aliases or revision changes must
    /// obtain their own policy; they cannot modify another frozen model profile.
    pub fn with_context_budget_policy(
        mut self,
        model: &ModelIdentity,
        policy: NativeContextBudgetPolicy,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        if &self.model != model {
            return Err(UnsupportedExecutionProfile);
        }
        policy.validate()?;
        self.context_budget_policy = policy;
        Ok(self)
    }
}

#[cfg(test)]
#[path = "context_budget/tests.rs"]
mod tests;
