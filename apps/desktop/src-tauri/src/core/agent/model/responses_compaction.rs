//! Fixed Codex compact_remote_v2.rs collector, e363b08c9175ac1cbe5893615dd2cb9ddf95043b.
//! Copyright 2025 OpenAI. Adapted under Apache-2.0. No candidate installation here.
use super::*;
use research_agent_core::context::checkpoint::{ContextCheckpoint, StoredMessage};
use research_agent_core::{
    context::compaction_strategy::{
        CompactionBinding, CompactionCandidate, CompactionFailure, CompactionFuture,
        CompactionKind, CompactionOutput, CompactionRequest, CompactionRetention,
        CompactionStrategy,
    },
    model::{FinishReason, ModelUsage},
};

pub(crate) fn validate_v2_output(output: &CompactionOutput) -> Result<(), CompactionFailure> {
    let CompactionOutput::RemoteOpaque(item) = output else {
        return Err(CompactionFailure::InvalidOutput);
    };
    if !matches!(
        item["type"].as_str(),
        Some("compaction" | "compaction_summary")
    ) || item
        .get("id")
        .is_some_and(|id| !id.is_null() && !id.is_string())
        || !item["encrypted_content"].is_string()
    {
        return Err(CompactionFailure::InvalidOutput);
    }
    Ok(())
}

/// Explicitly injected native capability. A Responses-compatible URL alone is
/// never sufficient to select this strategy. Credentials stay in the transport.
pub struct ResponsesV2CompactionStrategy {
    transport: Arc<ResponsesTransport>,
    binding: CompactionBinding,
    retry: research_agent_core::resilience::RetryPolicy,
    retained_text: Arc<dyn research_agent_core::context::retained_text::RetainedTextPolicy>,
    accounting: Option<Arc<research_agent_core::context::accounting::ContextAccounting>>,
}
impl ResponsesV2CompactionStrategy {
    pub fn new(
        transport: Arc<ResponsesTransport>,
        binding: CompactionBinding,
    ) -> Result<Self, CompactionFailure> {
        binding.validate()?;
        if binding.kind != CompactionKind::RemoteOpaque
            || binding.model.provider_id != transport.config.provider_id
        {
            return Err(CompactionFailure::Incompatible);
        }
        Ok(Self {
            transport,
            binding,
            retained_text: Arc::new(
                research_agent_core::context::retained_text::Utf8RetainedTextPolicy {
                    max_tokens: 64_000,
                },
            ),
            retry: research_agent_core::resilience::RetryPolicy {
                max_attempts: 1,
                base_delay_ms: 0,
                max_delay_ms: 0,
            },
            accounting: None,
        })
    }
    pub fn with_input_accounting(
        mut self,
        accounting: Arc<research_agent_core::context::accounting::ContextAccounting>,
    ) -> Result<Self, CompactionFailure> {
        if accounting.model != self.binding.model {
            return Err(CompactionFailure::Incompatible);
        }
        self.accounting = Some(accounting);
        Ok(self)
    }
    pub fn with_retained_text_policy(
        mut self,
        policy: Arc<dyn research_agent_core::context::retained_text::RetainedTextPolicy>,
    ) -> Result<Self, CompactionFailure> {
        if policy.max_tokens() == 0 {
            return Err(CompactionFailure::InvalidBudget);
        }
        self.retained_text = policy;
        Ok(self)
    }
    /// Explicit provider policy; this HTTPS adapter has no alternate transport.
    /// Pinned V2 caps provider stream retries at two (three total attempts).
    pub fn with_retry_policy(
        mut self,
        mut retry: research_agent_core::resilience::RetryPolicy,
    ) -> Result<Self, ModelError> {
        if retry.max_attempts == 0
            || retry.base_delay_ms > retry.max_delay_ms
            || retry.max_delay_ms > 30_000
        {
            return Err(ModelError::InvalidRequest);
        }
        retry.max_attempts = retry.max_attempts.min(3);
        self.retry = retry;
        Ok(self)
    }
}

/// One retained/canonical user message encodes as one Responses input item.
/// Assistant/tool prefixes and legacy full windows have different contracts.
pub(crate) fn retained_v2_inputs(checkpoint: &ContextCheckpoint) -> Result<usize, ModelError> {
    let messages = checkpoint.retained.iter().chain(
        checkpoint
            .canonical_context
            .iter()
            .flat_map(|context| &context.messages),
    );
    if messages
        .clone()
        .any(|message| !matches!(message, StoredMessage::User { .. }))
    {
        return Err(ModelError::UnsupportedCapability);
    }
    Ok(messages.count())
}

impl ResponsesTransport {
    pub(super) fn validate_installed_window<'a>(
        &'a self,
        checkpoint: &'a ContextCheckpoint,
        request: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        Box::pin(async move {
            checkpoint
                .validate(MAX_REQUEST)
                .map_err(|_| ModelError::InvalidRequest)?;
            if checkpoint.binding.model != request.model
                || request.model.provider_id != self.config.provider_id
            {
                return Err(ModelError::UnsupportedCapability);
            }
            let mut body = self.request_body(request, false)?;
            let has_native_history = request.messages.iter().any(|message| {
                matches!(message, research_agent_core::model::ModelMessage::CanonicalOutput(item) if item.native.is_some())
            });
            let opaque_window = matches!(
                checkpoint.output,
                CompactionOutput::RemoteOpaque(_) | CompactionOutput::RemoteWindow(_)
            );
            let lease = if has_native_history || opaque_window {
                let lease = self.acquire_chatgpt_lease().await?;
                if let Some(lease) = &lease {
                    lease.check().map_err(|_| ModelError::Authentication)?;
                }
                lease
            } else {
                None
            };
            let account = lease
                .as_ref()
                .map_or(self.config.credential_scope_id, |lease| lease.scope_id());
            if has_native_history {
                if account.is_nil() {
                    return Err(ModelError::Authentication);
                }
                self.validate_native_history_scope(request, account)?;
            }
            match &checkpoint.output {
                CompactionOutput::TextSummary(_) | CompactionOutput::ResetWindow
                    if checkpoint.credential_scope_id.is_none() => {}
                CompactionOutput::RemoteOpaque(_) | CompactionOutput::RemoteWindow(_) => {
                    if account.is_nil() || checkpoint.credential_scope_id != Some(account) {
                        return Err(ModelError::Authentication);
                    }
                    let owner = super::super::responses_continuation::ResponsesContinuation::new(
                        checkpoint.window_id,
                        request.model.clone(),
                        MAX_REQUEST,
                    )?;
                    match &checkpoint.output {
                        CompactionOutput::RemoteOpaque(item) => owner.bind_compaction_sync(
                            account,
                            &request.model,
                            item,
                            retained_v2_inputs(checkpoint)?,
                        )?,
                        CompactionOutput::RemoteWindow(items) => owner
                            .bind_window_with_canonical_sync(
                                account,
                                &request.model,
                                items,
                                if checkpoint.schema_version == 4 {
                                    checkpoint
                                        .canonical_context
                                        .as_ref()
                                        .map_or(0, |context| context.messages.len())
                                } else {
                                    0
                                },
                            )?,
                        _ => return Err(ModelError::UnsupportedCapability),
                    }
                    let input = body["input"].as_array().ok_or(ModelError::InvalidRequest)?;
                    let authority = request
                        .messages
                        .iter()
                        .take_while(|message| {
                            matches!(message, research_agent_core::model::ModelMessage::System(_))
                        })
                        .count();
                    body["input"] = Value::Array(owner.expand_compaction(input, Some(authority))?);
                }
                _ => return Err(ModelError::UnsupportedCapability),
            }
            if serde_json::to_vec(&body)
                .map_err(|_| ModelError::InvalidRequest)?
                .len()
                > MAX_REQUEST
            {
                return Err(ModelError::BudgetExceeded);
            }
            Ok(())
        })
    }
}
impl CompactionStrategy for ResponsesV2CompactionStrategy {
    fn input_accounting(
        &self,
    ) -> Option<Arc<research_agent_core::context::accounting::ContextAccounting>> {
        self.accounting.clone()
    }
    fn retained_text_policy(
        &self,
    ) -> Option<Arc<dyn research_agent_core::context::retained_text::RetainedTextPolicy>> {
        Some(self.retained_text.clone())
    }
    fn binding(&self) -> CompactionBinding {
        self.binding.clone()
    }
    fn retention_policy(&self) -> Option<CompactionRetention> {
        Some(CompactionRetention::RecentUserMessages)
    }
    fn validate_output(&self, output: &CompactionOutput) -> Result<(), CompactionFailure> {
        validate_v2_output(output)
    }
    fn compact(&self, request: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        Box::pin(async move {
            let mut attempts = 0;
            loop {
                attempts += 1;
                match self.compact_once(request.clone()).await {
                    Ok(candidate) => return Ok(candidate),
                    Err(CompactionFailure::Model(error)) => {
                        match self.retry.decide(error, attempts) {
                            research_agent_core::resilience::RetryDecision::Retry { delay_ms } => {
                                // The task owns this future; dropping it also
                                // cancels backoff. No partial candidate survives.
                                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            }
                            research_agent_core::resilience::RetryDecision::Stop => {
                                return Err(error.into());
                            }
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        })
    }
}
impl ResponsesV2CompactionStrategy {
    fn compact_once(
        &self,
        request: CompactionRequest,
    ) -> CompactionFuture<'_, CompactionCandidate> {
        Box::pin(async move {
            if request.binding != self.binding || request.source.model != self.binding.model {
                return Err(CompactionFailure::Incompatible);
            }
            let mut source = request.source;
            source.request_id = request.operation_id;
            let mut stream = self
                .transport
                .start_owned_inner(source, true, false, self.accounting.clone())
                .await?;
            let mut response_id = None;
            let mut usage = ModelUsage::default();
            let mut completed = false;
            while let Some(event) = stream.next().await? {
                match event {
                    ModelStreamEvent::Started {
                        response_id: id,
                        actual_model,
                    } => {
                        if actual_model.provider_id != self.binding.model.provider_id {
                            return Err(CompactionFailure::Incompatible);
                        }
                        response_id = Some(id);
                    }
                    ModelStreamEvent::StartedWithoutModel { response_id: id } => {
                        response_id = Some(id)
                    }
                    ModelStreamEvent::ModelIdentityObserved(actual_model) => {
                        if actual_model.provider_id != self.binding.model.provider_id {
                            return Err(CompactionFailure::Incompatible);
                        }
                    }
                    ModelStreamEvent::Usage(observed) => usage = observed,
                    ModelStreamEvent::Completed {
                        reason: FinishReason::Stop,
                        ..
                    } => {
                        completed = true;
                        break;
                    }
                    _ => return Err(CompactionFailure::InvalidOutput),
                }
            }
            if !completed || response_id.is_none() {
                return Err(ModelError::IncompleteStream.into());
            }
            let output = CompactionOutput::RemoteOpaque(
                stream
                    .compaction_output
                    .take()
                    .ok_or(ModelError::InvalidStream)?,
            );
            self.validate_output(&output)?;
            // Enforce the persisted envelope size before returning even when the
            // native caller invokes the strategy directly instead of prepare_compaction.
            if serde_json::to_vec(&output)
                .map_err(|_| CompactionFailure::InvalidOutput)?
                .len()
                > request.max_result_bytes
            {
                return Err(CompactionFailure::OutputBudget);
            }
            Ok(CompactionCandidate {
                binding: self.binding.clone(),
                credential_scope_id: Some(stream.credential_scope_id),
                output,
                usage,
                response_id,
            })
        })
    }
}
