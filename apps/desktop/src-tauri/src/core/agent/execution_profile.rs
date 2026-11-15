//! Native execution policy shared by research hosts. No renderer capability claims.
#[path = "execution_profile/accounting_transport.rs"]
mod accounting_transport;
#[path = "execution_profile/context_budget.rs"]
mod context_budget;
#[path = "execution_profile/hook_context.rs"]
mod hook_context;
#[path = "execution_profile/model_policy.rs"]
mod model_policy;
pub use model_policy::{
    execution_profile, execution_profile_for_host, execution_profile_with_compaction,
};
#[path = "execution_profile/model_transition.rs"]
pub(crate) mod model_transition;
#[path = "execution_profile/token_budget.rs"]
pub(crate) mod token_budget;
pub use context_budget::NativeContextBudgetPolicy;
#[path = "execution_profile/compaction.rs"]
pub(crate) mod compaction;
use super::model::{
    AgentModelPreparationError, ChatRequestPolicy, OutputTokenParameter, PreparedAgentModel,
};
use crate::{
    agent::repository::responses_state::{ResponsesStateScope, SqliteResponsesState},
    storage::AppStore,
};
pub use compaction::NativeCompactionPolicy;
use research_agent_core::{
    model::{ModelCapabilities, ModelTransport, StreamLimits},
    protocol::ModelIdentity,
    runner::RunLimits,
    submission::SubmissionRecord,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentExecutionMode {
    DeepSeekNonThinking,
    ChatGptDefault,
}

/// Derive the reviewed execution mode from a frozen model selection. The
/// provider/protocol scope remains enforced by `execution_profile`; callers
/// must not infer a capability class independently in each host adapter.
pub fn execution_mode_for_model(
    model: &ModelIdentity,
    reasoning_effort: Option<&str>,
) -> AgentExecutionMode {
    if model.provider_id == "deepseek" && reasoning_effort.is_none() {
        AgentExecutionMode::DeepSeekNonThinking
    } else {
        AgentExecutionMode::ChatGptDefault
    }
}
/// Native host supplies a verified profile, never a renderer-provided capability claim.
#[derive(Clone)]
pub struct AgentExecutionProfile {
    pub(crate) hook_context_bytes: usize,
    pub(crate) reasoning_summary:
        Option<super::model::responses_reasoning::ResponsesReasoningSummary>,
    pub(crate) chat_developer_model: Option<ModelIdentity>,
    pub(crate) compact_prompt: Option<String>,
    pub(crate) token_budget_settings: Option<token_budget::ResolvedTokenBudgetSettings>,
    pub(crate) comp_hash: Option<String>,
    pub(crate) context_budget_policy: NativeContextBudgetPolicy,
    pub(crate) compaction_policy: compaction::NativeCompactionPolicy,
    pub request_policy: crate::agent::model::ChatRequestPolicy,
    pub model: ModelIdentity,
    pub capabilities: ModelCapabilities,
    pub token_parameter: OutputTokenParameter,
    pub max_output_tokens: u32,
    pub max_input_bytes: usize,
    pub limits: RunLimits,
}
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("unsupported execution model or mode")]
pub struct UnsupportedExecutionProfile;

impl AgentExecutionProfile {
    /// Apply only the app-owned feature value already frozen in this receipt.
    pub(crate) fn with_submission_compaction(
        mut self,
        input: &research_agent_core::submission::SubmissionInput,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        if self.model != input.model {
            return Err(UnsupportedExecutionProfile);
        }
        if let Some(v2) = input.remote_compaction_v2 {
            if self.model.provider_id != "openai" {
                return Err(UnsupportedExecutionProfile);
            }
            self.compaction_policy = if v2 {
                NativeCompactionPolicy::ChatGptResponsesV2
            } else {
                NativeCompactionPolicy::ChatGptResponsesFullWindow
            };
        }
        Ok(self)
    }
    /// Trusted experimental opt-in takes precedence over the profile's summary
    /// policy. Exact identity is checked before clearing summary-only settings.
    pub fn with_token_budget_reset(
        self,
        model: &ModelIdentity,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        self.with_token_budget_settings(model, Default::default())
    }
    /// Configuration is resolved before freezing and applies only to this exact
    /// provider/model revision. Renderer input cannot enable this policy.
    pub fn with_token_budget_settings(
        mut self,
        model: &ModelIdentity,
        settings: research_agent_core::context::token_budget::TokenBudgetSettings,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        if &self.model != model {
            return Err(UnsupportedExecutionProfile);
        }
        self.token_budget_settings = Some(
            settings
                .try_into()
                .map_err(|_| UnsupportedExecutionProfile)?,
        );
        self.compaction_policy = NativeCompactionPolicy::TokenBudgetReset;
        self.compact_prompt = None;
        Ok(self)
    }
    /// Read the strategy composed with the frozen native transport. Host callers
    /// cannot infer remote capability from the transport's encoder support.
    pub(crate) fn compaction_strategy(
        &self,
        transport: Arc<dyn ModelTransport>,
    ) -> Result<
        Arc<dyn research_agent_core::context::compaction_strategy::CompactionStrategy>,
        AgentModelPreparationError,
    > {
        let strategy = match transport.prepared_compaction_strategy() {
            Some(strategy) => strategy,
            None if self.compaction_policy == NativeCompactionPolicy::TokenBudgetReset => {
                compaction::reset_strategy(self.model.clone())?
            }
            None if matches!(
                self.compaction_policy,
                compaction::NativeCompactionPolicy::Text
            ) =>
            {
                compaction::text_strategy_with_prompt(
                    transport.clone(),
                    self.model.clone(),
                    self.compact_prompt.as_deref(),
                )?
            }
            None => {
                return Err(research_agent_core::model::ModelError::UnsupportedCapability.into());
            }
        };
        if strategy.binding() != self.compaction_policy.binding(self.model.clone())
            || (self.compaction_policy != NativeCompactionPolicy::TokenBudgetReset
                && !transport.supports_compaction_kind(strategy.binding().kind))
        {
            return Err(research_agent_core::model::ModelError::UnsupportedCapability.into());
        }
        if self.compaction_policy == NativeCompactionPolicy::Text {
            // Compose textual summary I/O with the native accounting and live
            // authority decorators, rather than the raw inner wire transport.
            return compaction::text_strategy_with_prompt(
                transport,
                self.model.clone(),
                strategy.text_compaction_prompt(),
            );
        }
        Ok(strategy)
    }
    /// Credentials and continuation state are scoped to the frozen submission.
    /// Host binding/disclosure checks must precede this constructor; constructing
    /// a transport does not start inference or grant any tool permissions.
    #[cfg(test)]
    pub(crate) fn transport(
        &self,
        store: &AppStore,
        record: &SubmissionRecord,
        prepared: PreparedAgentModel,
    ) -> Result<Arc<dyn ModelTransport>, AgentModelPreparationError> {
        self.transport_inner(store, record, prepared, None, None)
    }
    pub(crate) async fn transport_with_model_history(
        &self,
        store: &AppStore,
        record: &SubmissionRecord,
        prepared: PreparedAgentModel,
        models: Option<super::model::AgentModelResolver>,
        host: super::model::AgentHost,
    ) -> Result<Arc<dyn ModelTransport>, AgentModelPreparationError> {
        let history = models.map(|models| model_transition::ModelHistory {
            store: store.clone(),
            models,
            host,
        });
        let profile = self.clone();
        let store = store.clone();
        let record = record.clone();
        tokio::task::spawn_blocking(move || {
            profile.transport_inner(&store, &record, prepared, history, None)
        })
        .await
        .map_err(|_| AgentModelPreparationError::Worker)?
    }
    fn transport_inner(
        &self,
        store: &AppStore,
        record: &SubmissionRecord,
        prepared: PreparedAgentModel,
        history: Option<model_transition::ModelHistory>,
        frozen: Option<super::repository::context_policy::FrozenContextPolicy>,
    ) -> Result<Arc<dyn ModelTransport>, AgentModelPreparationError> {
        let restored = match &frozen {
            Some(_) => None,
            None => super::repository::context_policy::SqliteContextPolicy::restored(
                &store.shared_agent_connection(),
                record,
            )?,
        };
        if self.model != record.input.model
            || prepared.identity() != &record.input.model
            || prepared.reasoning_effort() != record.input.reasoning_effort.as_deref()
            || prepared.service_tier() != record.input.service_tier.as_deref()
            || record
                .input
                .reasoning_effort
                .as_ref()
                .is_some_and(|effort| !self.capabilities.reasoning_efforts.contains(effort))
            || self.max_output_tokens == 0
            || self.max_input_bytes == 0
            || !self.capabilities.tools
            || (prepared.is_chatgpt()
                && !matches!(self.request_policy, ChatRequestPolicy::ProviderDefault))
            || (!prepared.is_chatgpt()
                && !matches!(
                    self.compaction_policy,
                    NativeCompactionPolicy::Text | NativeCompactionPolicy::TokenBudgetReset
                ))
        {
            return Err(AgentModelPreparationError::Transport(
                research_agent_core::model::ModelError::UnsupportedCapability,
            ));
        }
        if self
            .chat_developer_model
            .as_ref()
            .is_some_and(|model| model != &self.model || prepared.is_chatgpt())
        {
            return Err(research_agent_core::model::ModelError::UnsupportedCapability.into());
        }
        let restored = restored.filter(|saved| {
            saved.binding == self.compaction_policy.binding(self.model.clone())
                && saved.max_output_tokens == self.max_output_tokens
                && saved.chat_developer_model == self.chat_developer_model
                && saved.authentication
                    == if prepared.is_chatgpt() {
                        super::repository::context_policy::ContextAuthentication::CodexBackend
                    } else {
                        super::repository::context_policy::ContextAuthentication::ApiKey
                    }
        });
        let budget = if let Some(frozen) = frozen.as_ref().or(restored.as_ref()) {
            frozen.validate()?;
            let auth = if prepared.is_chatgpt() {
                super::repository::context_policy::ContextAuthentication::CodexBackend
            } else {
                super::repository::context_policy::ContextAuthentication::ApiKey
            };
            if frozen.binding != self.compaction_policy.binding(self.model.clone())
                || frozen.authentication != auth
                || frozen.max_output_tokens != self.max_output_tokens
                || frozen.chat_developer_model != self.chat_developer_model
            {
                return Err(research_agent_core::model::ModelError::UnsupportedCapability.into());
            }
            frozen.budget
        } else {
            let mut policy = self.context_budget_policy;
            if policy == NativeContextBudgetPolicy::REVIEWED_DEFAULT
                && let Some((percent, trigger)) = prepared.catalog_budget()
            {
                policy.effective_percent = percent;
                policy.configured_trigger_tokens = trigger;
            }
            policy
                .resolve(prepared.context_window_tokens(), self.max_output_tokens)
                .map_err(|_| {
                    AgentModelPreparationError::Transport(
                        research_agent_core::model::ModelError::InvalidRequest,
                    )
                })?
        };
        let frozen_policy = super::repository::context_policy::SqliteContextPolicy::new(
            store.shared_agent_connection(),
            record,
            frozen.unwrap_or_else(|| super::repository::context_policy::FrozenContextPolicy {
                reasoning_summary: self.reasoning_summary,
                chat_developer_model: self.chat_developer_model.clone(),
                schema_version: 1,
                compact_prompt: self.compact_prompt.clone(),
                token_budget_settings: self.token_budget_settings.clone(),
                tool_truncation_policy: restored.as_ref().map_or_else(
                    || prepared.catalog_truncation_policy(),
                    |saved| saved.tool_truncation_policy,
                ),
                binding: self.compaction_policy.binding(self.model.clone()),
                budget,
                max_output_tokens: self.max_output_tokens,
                scope: self.context_budget_policy.scope,
                unknown_window_trigger_tokens: self.context_budget_policy.configured_trigger_tokens,
                comp_hash: self.comp_hash.clone(),
                authentication: if prepared.is_chatgpt() {
                    super::repository::context_policy::ContextAuthentication::CodexBackend
                } else {
                    super::repository::context_policy::ContextAuthentication::ApiKey
                },
            }),
        )?;
        let snapshot = frozen_policy.snapshot();
        if snapshot.reasoning_summary.is_some()
            && !context_budget::supports_reasoning_summary(&self.model)
        {
            return Err(research_agent_core::model::ModelError::UnsupportedCapability.into());
        }
        let estimator: Arc<dyn research_agent_core::context::accounting::ContextTokenEstimator> =
            if prepared.is_chatgpt() {
                Arc::new(super::model::responses_accounting::ResponsesContextEstimator)
            } else {
                Arc::new(research_agent_core::context::accounting::Utf8JsonTokenEstimator)
            };
        let estimator: Arc<dyn research_agent_core::context::accounting::ContextTokenEstimator> =
            Arc::new(
                research_agent_core::context::accounting::ServerObservedEstimator::new(estimator),
            );
        let estimator: Arc<dyn research_agent_core::context::accounting::ContextTokenEstimator> =
            match snapshot.scope {
                research_agent_core::context::accounting::AutoCompactionScope::Total => estimator,
                research_agent_core::context::accounting::AutoCompactionScope::BodyAfterPrefix => {
                    Arc::new(super::repository::prefill::SqlitePrefillEstimator::new(
                        store.shared_agent_connection(),
                        record.conversation_id,
                        &self.model,
                        estimator,
                    )?)
                }
            };
        let token_budget = snapshot.token_budget_policy()?.map(Arc::new);
        let prompt = snapshot.compact_prompt;
        let accounting = Arc::new(
            research_agent_core::context::accounting::ContextAccounting {
                model: self.model.clone(),
                budget,
                unknown_window_trigger_tokens: snapshot.unknown_window_trigger_tokens,
                estimator,
            },
        );
        let (inner, strategy): (Arc<dyn ModelTransport>, _) = if prepared.is_chatgpt() {
            let state = SqliteResponsesState::new(
                store.shared_agent_connection(),
                ResponsesStateScope {
                    conversation_id: record.conversation_id,
                    turn_id: record.turn_id,
                    context_snapshot_id: record.context_snapshot_id,
                    model: record.input.model.clone(),
                },
                32 * 1024 * 1024,
            )?
            .with_native_read_policy(self.compaction_policy);
            let inner = Arc::new(
                prepared
                    .into_chatgpt_transport(self.capabilities.clone(), Arc::new(state))?
                    .with_reasoning_summary(self.model.clone(), snapshot.reasoning_summary)?,
            );
            let strategy = if self.compaction_policy == NativeCompactionPolicy::Text {
                compaction::text_strategy_with_prompt(
                    inner.clone(),
                    self.model.clone(),
                    prompt.as_deref(),
                )?
            } else {
                self.compaction_policy.responses_strategy_with_accounting(
                    inner.clone(),
                    self.model.clone(),
                    accounting.clone(),
                )?
            };
            let strategy: Arc<
                dyn research_agent_core::context::compaction_strategy::CompactionStrategy,
            > = Arc::new(compaction::NativeReadStrategy {
                inner: strategy,
                policy: self.compaction_policy,
            });
            (inner, strategy)
        } else {
            let inner: Arc<dyn ModelTransport> =
                Arc::new(self.chat_transport(prepared, snapshot.chat_developer_model.as_ref())?);
            let strategy = if self.compaction_policy == NativeCompactionPolicy::TokenBudgetReset {
                compaction::reset_strategy(self.model.clone())?
            } else {
                compaction::text_strategy_with_prompt(
                    inner.clone(),
                    self.model.clone(),
                    prompt.as_deref(),
                )?
            };
            (inner, strategy)
        };
        Ok(Arc::new(accounting_transport::AccountingTransport::new(
            frozen_policy,
            history,
            inner,
            accounting,
            strategy,
            token_budget,
        )))
    }
}

#[cfg(test)]
#[path = "execution_profile/reading_budget_tests.rs"]
mod reading_budget_tests;

#[cfg(test)]
#[path = "execution_profile/strategy_tests.rs"]
mod strategy_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_catalog_requires_exact_provider_model_and_explicit_mode() {
        let mut model = ModelIdentity {
            provider_id: "deepseek".into(),
            model_id: "deepseek-v4-pro".into(),
            revision: None,
        };
        let selected = execution_profile(&model, AgentExecutionMode::DeepSeekNonThinking).unwrap();
        assert!(selected.capabilities.tools);
        assert!(!selected.capabilities.vision);
        assert!(selected.capabilities.reasoning_efforts.is_empty());
        for (provider, name) in [
            ("qwen", "deepseek-v4-pro"),
            ("deepseek", "deepseek-v4-pro-new"),
            ("openai", "gpt-5"),
        ] {
            model.provider_id = provider.into();
            model.model_id = name.into();
            assert!(matches!(
                execution_profile(&model, AgentExecutionMode::DeepSeekNonThinking),
                Err(UnsupportedExecutionProfile)
            ));
        }
    }

    #[test]
    fn execution_mode_inference_is_shared_across_child_hosts() {
        let deepseek = ModelIdentity {
            provider_id: "deepseek".into(),
            model_id: "deepseek-v4-flash".into(),
            revision: None,
        };
        assert!(matches!(
            execution_mode_for_model(&deepseek, None),
            AgentExecutionMode::DeepSeekNonThinking
        ));
        assert!(matches!(
            execution_mode_for_model(&deepseek, Some("medium")),
            AgentExecutionMode::ChatGptDefault
        ));
        assert!(matches!(
            execution_mode_for_model(
                &ModelIdentity {
                    provider_id: "openai".into(),
                    model_id: "gpt-5.6-terra".into(),
                    revision: None,
                },
                None,
            ),
            AgentExecutionMode::ChatGptDefault
        ));
    }

    #[test]
    fn chatgpt_profile_is_a_protocol_bound_and_transport_requires_exact_catalog_metadata() {
        for (provider, name, accepted) in [
            ("openai", "gpt-5.6-terra", true),
            ("openai", "gpt-5.6-sol", true),
            ("openai", "gpt-5.6-terra-new", true),
            ("openai", "", false),
            ("deepseek", "gpt-5.6-terra", false),
        ] {
            let model = ModelIdentity {
                provider_id: provider.into(),
                model_id: name.into(),
                revision: None,
            };
            let selected = execution_profile(&model, AgentExecutionMode::ChatGptDefault);
            assert_eq!(selected.is_ok(), accepted);
            if let Ok(profile) = selected {
                assert!(matches!(
                    profile.request_policy,
                    ChatRequestPolicy::ProviderDefault
                ));
                assert!(profile.capabilities.tools);
                assert!(!profile.capabilities.vision);
                assert_eq!(
                    profile.capabilities.reasoning_efforts,
                    ["none", "low", "medium", "high", "xhigh", "max"]
                );
            }
        }
    }

    #[test]
    fn reading_host_enables_vision_only_for_the_reviewed_reading_route() {
        let model = ModelIdentity {
            provider_id: "deepseek".into(),
            model_id: "deepseek-v4-pro".into(),
            revision: None,
        };
        assert!(
            execution_profile_for_host(
                &model,
                AgentExecutionMode::DeepSeekNonThinking,
                crate::agent::model::AgentHost::Reading,
            )
            .unwrap()
            .capabilities
            .vision
        );
        assert!(
            !execution_profile_for_host(
                &model,
                AgentExecutionMode::DeepSeekNonThinking,
                crate::agent::model::AgentHost::Writing,
            )
            .unwrap()
            .capabilities
            .vision
        );
    }
}

#[path = "execution_profile/chat_role.rs"]
mod chat_role;
