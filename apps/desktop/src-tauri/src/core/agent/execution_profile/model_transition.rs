//! Previous-model handoff through the frozen native supplier contract.
use super::super::{
    model::{AgentHost, AgentModelResolver, AgentModelSelection},
    repository::context_policy::{FrozenContextPolicy, SqliteContextPolicy},
};
use super::*;
use research_agent_core::{
    context::{accounting::AutoCompactionScope, compaction_strategy::*},
    model::*,
};

pub(super) struct ModelHistory {
    pub store: AppStore,
    pub models: AgentModelResolver,
    pub host: AgentHost,
}

// Keep authorization and transport failures distinct before deciding whether
// a provider may use another model. No credential details cross this boundary.
fn preparation_error(error: AgentModelPreparationError) -> ModelError {
    match error {
        AgentModelPreparationError::Transport(error) => error,
        AgentModelPreparationError::Credentials
        | AgentModelPreparationError::ChatGptTransportUnavailable => ModelError::Authentication,
        AgentModelPreparationError::Disabled | AgentModelPreparationError::UnsupportedProtocol => {
            ModelError::UnsupportedCapability
        }
        AgentModelPreparationError::Settings | AgentModelPreparationError::Worker => {
            ModelError::Unavailable
        }
    }
}

pub(super) fn needs_old_model(
    previous: &FrozenContextPolicy,
    current: &FrozenContextPolicy,
    active: Option<u64>,
) -> Result<bool, ModelError> {
    previous.validate()?;
    current.validate()?;
    if previous
        .comp_hash
        .as_ref()
        .zip(current.comp_hash.as_ref())
        .is_some_and(|(old, new)| old != new)
    {
        return Ok(true);
    }
    if previous.binding.model.model_id == current.binding.model.model_id {
        return Ok(false);
    }
    let Some((old, new)) = previous.budget.zip(current.budget) else {
        return Ok(false);
    };
    let effective = |budget: CompactionBudget| {
        u128::from(budget.context_window_tokens) * u128::from(budget.effective_percent) / 100
    };
    if effective(new) >= effective(old) {
        return Ok(false);
    }
    Ok(active.is_some_and(|tokens| {
        let hard_window = effective(new);
        let total = u128::from(tokens);
        let auto_limit = (u128::from(new.context_window_tokens) * 90 / 100).min(u128::from(
            new.configured_trigger_tokens.unwrap_or(u64::MAX),
        ));
        total >= hard_window || (current.scope == AutoCompactionScope::Total && total > auto_limit)
    }))
}

fn frozen_native_policy(binding: &CompactionBinding) -> Result<NativeCompactionPolicy, ModelError> {
    for policy in [
        NativeCompactionPolicy::Text,
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        if policy.binding(binding.model.clone()) == *binding {
            return Ok(policy);
        }
    }
    Err(ModelError::UnsupportedCapability)
}

fn allows_handoff(previous: &FrozenContextPolicy, current: &FrozenContextPolicy) -> bool {
    use super::super::repository::context_policy::ContextAuthentication;
    let source = &previous.binding;
    let target = &current.binding;
    let (Ok(source_policy), Ok(target_policy)) =
        (frozen_native_policy(source), frozen_native_policy(target))
    else {
        return false;
    };
    (source_policy == NativeCompactionPolicy::Text && target_policy == NativeCompactionPolicy::Text)
        || (matches!(
            source_policy,
            NativeCompactionPolicy::ChatGptResponsesV2
                | NativeCompactionPolicy::ChatGptResponsesFullWindow
        ) && matches!(
            target_policy,
            NativeCompactionPolicy::ChatGptResponsesV2
                | NativeCompactionPolicy::ChatGptResponsesFullWindow
        ) && source.model.provider_id == "openai"
            && target.model.provider_id == "openai"
            && previous.authentication == ContextAuthentication::CodexBackend
            && current.authentication == ContextAuthentication::CodexBackend)
}

impl ModelHistory {
    pub async fn select(
        &self,
        policy: &SqliteContextPolicy,
        turn: uuid::Uuid,
        model: &ModelIdentity,
        active: Option<u64>,
    ) -> Result<Option<Arc<dyn CompactionStrategy>>, ModelError> {
        let Some((record, previous)) = policy.previous(turn, model).await? else {
            return Ok(None);
        };
        let current = policy.snapshot();
        if !needs_old_model(&previous, &current, active)? {
            return Ok(None);
        }
        if !allows_handoff(&previous, &current) {
            return Err(ModelError::UnsupportedCapability);
        }
        let mode = execution_mode_for_model(
            &record.input.model,
            record.input.reasoning_effort.as_deref(),
        );
        let mut profile = execution_profile_for_host(&record.input.model, mode, self.host)
            .map_err(|_| ModelError::UnsupportedCapability)?;
        // Codex's previous-model context retains the current turn's feature
        // selection. Only the model and its frozen limits come from the source.
        profile.compaction_policy = frozen_native_policy(&current.binding)?;
        profile.max_output_tokens = previous.max_output_tokens;
        profile.context_budget_policy.scope = previous.scope;
        profile.context_budget_policy.configured_trigger_tokens =
            previous.unknown_window_trigger_tokens;
        profile.comp_hash = previous.comp_hash.clone();
        profile.chat_developer_model = previous.chat_developer_model.clone();
        let prepared = self
            .models
            .prepare(
                self.host,
                Some(AgentModelSelection {
                    provider_id: record.input.model.provider_id.clone(),
                    model_id: record.input.model.model_id.clone(),
                    reasoning_effort: record.input.reasoning_effort.clone(),
                    service_tier: record.input.service_tier.clone(),
                }),
            )
            .await
            .map_err(preparation_error)?;
        let mut producer_view = previous.clone();
        producer_view.binding = profile
            .compaction_policy
            .binding(record.input.model.clone());
        let transport = profile
            .transport_inner(&self.store, &record, prepared, None, Some(producer_view))
            .map_err(preparation_error)?;
        let strategy = profile
            .compaction_strategy(transport.clone())
            .map_err(preparation_error)?;
        Ok(Some(Arc::new(PreviousModelStrategy::compose(
            strategy,
            transport,
            current.binding,
            record.input.reasoning_effort,
            previous.max_output_tokens,
            previous.binding.kind != CompactionKind::TextSummary,
        )?)))
    }
}

pub(crate) struct PreviousModelStrategy {
    old: Arc<dyn CompactionStrategy>,
    transport: Arc<dyn ModelTransport>,
    target: CompactionBinding,
    reasoning: Option<String>,
    max_output_tokens: u32,
}
impl PreviousModelStrategy {
    #[cfg(test)]
    pub fn new(
        old: Arc<dyn CompactionStrategy>,
        transport: Arc<dyn ModelTransport>,
        target: CompactionBinding,
        reasoning: Option<String>,
        max_output_tokens: u32,
    ) -> Result<Self, ModelError> {
        Self::compose(old, transport, target, reasoning, max_output_tokens, false)
    }
    fn compose(
        old: Arc<dyn CompactionStrategy>,
        transport: Arc<dyn ModelTransport>,
        target: CompactionBinding,
        reasoning: Option<String>,
        max_output_tokens: u32,
        remote: bool,
    ) -> Result<Self, ModelError> {
        let source = old.binding();
        source.validate().map_err(|_| ModelError::InvalidRequest)?;
        target.validate().map_err(|_| ModelError::InvalidRequest)?;
        if max_output_tokens == 0
            || (!remote && source.kind != CompactionKind::TextSummary)
            || (remote
                && (source.kind == CompactionKind::TextSummary
                    || source.model.provider_id != "openai"
                    || target.model.provider_id != "openai"))
            || target.kind != source.kind
            || target.strategy_id != source.strategy_id
            || target.state_schema != source.state_schema
        {
            return Err(ModelError::UnsupportedCapability);
        }
        Ok(Self {
            old,
            transport,
            target,
            reasoning,
            max_output_tokens,
        })
    }
}
impl CompactionStrategy for PreviousModelStrategy {
    fn tool_output_policy(
        &self,
    ) -> Option<research_agent_core::model::tool_output::ToolOutputPolicy> {
        self.transport.tool_output_policy()
    }
    fn text_compaction_prompt(&self) -> Option<&str> {
        self.old.text_compaction_prompt()
    }
    fn input_accounting(
        &self,
    ) -> Option<Arc<research_agent_core::context::accounting::ContextAccounting>> {
        self.transport.context_accounting()
    }
    fn binding(&self) -> CompactionBinding {
        self.target.clone()
    }
    fn retention_policy(&self) -> Option<CompactionRetention> {
        self.old.retention_policy()
    }
    fn retained_text_policy(
        &self,
    ) -> Option<Arc<dyn research_agent_core::context::retained_text::RetainedTextPolicy>> {
        self.old.retained_text_policy()
    }
    fn execution_model(&self) -> ModelIdentity {
        self.old.execution_model()
    }
    fn allows_current_model_fallback(&self, error: &ModelError) -> bool {
        self.target.kind != CompactionKind::TextSummary
            && self.old.execution_model().model_id != self.target.model.model_id
            && matches!(
                error,
                ModelError::InvalidRequest
                    | ModelError::UnexpectedStatus(_)
                    | ModelError::ContextWindowExceeded
                    | ModelError::UsageLimitReached
                    | ModelError::ServerOverloaded
                    | ModelError::InternalServerError
            )
    }
    fn validate_output(&self, output: &CompactionOutput) -> Result<(), CompactionFailure> {
        self.old.validate_output(output)
    }
    fn compact(&self, mut request: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        Box::pin(async move {
            if request.binding != self.target || request.source.model != self.target.model {
                return Err(CompactionFailure::Incompatible);
            }
            request.binding = self.old.binding();
            request.source.model = request.binding.model.clone();
            request.source.reasoning_effort = self.reasoning.clone();
            request.source.max_output_tokens =
                request.source.max_output_tokens.min(self.max_output_tokens);
            self.transport.authorize_request(&request.source).await?;
            let mut candidate = prepare_compaction(self.old.as_ref(), request).await?;
            // This adapter declares the consuming codec. Account identity and
            // actual old producer survive independently in candidate/lifecycle.
            candidate.binding = self.target.clone();
            Ok(candidate)
        })
    }
}

#[cfg(test)]
#[path = "model_transition/tests.rs"]
mod tests;
