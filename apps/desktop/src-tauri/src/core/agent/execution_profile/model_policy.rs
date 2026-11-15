//! Provider/protocol upper bounds. Prepared native metadata admits exact models.
use super::*;
/// Reviewed protocol upper bound, not model-name pattern inference. ChatGPT's
/// exact membership and reasoning/vision intersection are enforced by prepared
/// native catalog metadata at transport composition. This template alone cannot
/// authorize a model request. Application budgets
/// below bound individual responses, not successful main-turn continuations or
/// advertised model token windows. Explicit child tasks add their own call budget.
pub fn execution_profile(
    model: &ModelIdentity,
    mode: AgentExecutionMode,
) -> Result<AgentExecutionProfile, UnsupportedExecutionProfile> {
    let policy = match mode {
        AgentExecutionMode::ChatGptDefault => NativeCompactionPolicy::ChatGptResponsesV2,
        AgentExecutionMode::DeepSeekNonThinking => NativeCompactionPolicy::Text,
    };
    execution_profile_with_compaction(model, mode, policy)
}

/// Trusted native composition input, deliberately absent from renderer IPC.
/// Choosing a policy does not bypass native catalog/credential admission.
pub fn execution_profile_with_compaction(
    model: &ModelIdentity,
    mode: AgentExecutionMode,
    policy: NativeCompactionPolicy,
) -> Result<AgentExecutionProfile, UnsupportedExecutionProfile> {
    if matches!(mode, AgentExecutionMode::DeepSeekNonThinking)
        && !matches!(
            policy,
            NativeCompactionPolicy::Text | NativeCompactionPolicy::TokenBudgetReset
        )
    {
        return Err(UnsupportedExecutionProfile);
    }
    let request_policy = match mode {
        AgentExecutionMode::DeepSeekNonThinking
            if model.provider_id == "deepseek"
                && matches!(
                    model.model_id.as_str(),
                    "deepseek-v4-pro" | "deepseek-v4-flash"
                ) =>
        {
            ChatRequestPolicy::DeepSeekNonThinking
        }
        AgentExecutionMode::ChatGptDefault
            if model.provider_id == "openai" && !model.model_id.trim().is_empty() =>
        {
            ChatRequestPolicy::ProviderDefault
        }
        _ => return Err(UnsupportedExecutionProfile),
    };
    Ok(AgentExecutionProfile {
        hook_context_bytes: 10_000,
        reasoning_summary: None,
        chat_developer_model: None,
        compact_prompt: None,
        token_budget_settings: None,
        comp_hash: None,
        context_budget_policy: NativeContextBudgetPolicy::REVIEWED_DEFAULT,
        compaction_policy: policy,
        model: model.clone(),
        request_policy,
        capabilities: ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            // Intersection of the pinned Codex catalog and Responses model
            // contract. `ultra` includes delegation, not just wire reasoning.
            reasoning_efforts: match mode {
                AgentExecutionMode::ChatGptDefault => {
                    ["none", "low", "medium", "high", "xhigh", "max"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect()
                }
                AgentExecutionMode::DeepSeekNonThinking => vec![],
            },
        },
        token_parameter: OutputTokenParameter::MaxTokens,
        max_output_tokens: 4096,
        // Context capacity is governed by the model's token budget. The old
        // fixed 128 KiB ceiling compacted or rejected valid large windows.
        max_input_bytes: usize::MAX,
        limits: RunLimits {
            model_calls: None,
            tool_calls: None,
            output_bytes: None,
            tool_output_bytes: 131072,
            response: StreamLimits {
                text_bytes: 131072,
                argument_bytes: 65536,
                tool_calls: 16,
            },
        },
    })
}

/// Host routing supplies the reviewed capability class. The reading route is
/// backed by a provider configured for `vision`; writing and notes remain
/// text-only even when they use the same transport adapter.
pub fn execution_profile_for_host(
    model: &ModelIdentity,
    mode: AgentExecutionMode,
    host: crate::agent::model::AgentHost,
) -> Result<AgentExecutionProfile, UnsupportedExecutionProfile> {
    let mut profile = execution_profile(model, mode)?;
    if matches!(host, crate::agent::model::AgentHost::Reading) {
        profile.capabilities.vision = true;
    }
    Ok(profile)
}
