//! Validated native TokenBudget configuration; prompt text is not debug output.
use research_agent_core::{
    context::token_budget::{TokenBudgetPolicy, TokenBudgetSettings},
    model::ModelError,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "TokenBudgetSettings", into = "TokenBudgetSettings")]
pub(crate) struct ResolvedTokenBudgetSettings(TokenBudgetSettings);
impl TryFrom<TokenBudgetSettings> for ResolvedTokenBudgetSettings {
    type Error = ModelError;
    fn try_from(settings: TokenBudgetSettings) -> Result<Self, Self::Error> {
        Ok(Self(
            TokenBudgetPolicy::resolve(settings)?.settings().clone(),
        ))
    }
}
impl From<ResolvedTokenBudgetSettings> for TokenBudgetSettings {
    fn from(settings: ResolvedTokenBudgetSettings) -> Self {
        settings.0
    }
}
impl std::fmt::Debug for ResolvedTokenBudgetSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedTokenBudgetSettings")
            .field(
                "reminder_threshold_tokens",
                &self.0.reminder_threshold_tokens,
            )
            .field(
                "fallback_buffer_tokens",
                &self.0.auto_compact_fallback_buffer_tokens,
            )
            .finish_non_exhaustive()
    }
}
impl ResolvedTokenBudgetSettings {
    pub fn policy(&self) -> Result<TokenBudgetPolicy, ModelError> {
        TokenBudgetPolicy::resolve(self.0.clone())
    }
}
