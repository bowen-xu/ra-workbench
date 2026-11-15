//! Fixed Codex codex-api/endpoint/compact.rs and common.rs CompactionInput,
//! e363b08c9175ac1cbe5893615dd2cb9ddf95043b, Apache-2.0, Copyright 2025 OpenAI.
//! Complete JSON output is a candidate window, never a V2 opaque item.
use super::*;
use research_agent_core::context::compaction_strategy::{
    CompactionBinding, CompactionCandidate, CompactionFailure, CompactionFuture, CompactionKind,
    CompactionOutput, CompactionRequest, CompactionStrategy,
};
use research_agent_core::model::ModelUsage;

pub(crate) fn validate_legacy_output(output: &CompactionOutput) -> Result<(), CompactionFailure> {
    let CompactionOutput::RemoteWindow(items) = output else {
        return Err(CompactionFailure::InvalidOutput);
    };
    if items.iter().any(|item| {
        item.as_object().is_none_or(|object| object.is_empty())
            || !super::compaction_output::valid(item)
    }) {
        return Err(CompactionFailure::InvalidOutput);
    }
    Ok(())
}

struct FrozenCompactHttpRequest {
    bytes: Vec<u8>,
    request_id: Uuid,
    metadata: Option<crate::agent::model::responses_metadata::ResponsesRequestMetadata>,
}

/// Native composition must explicitly select the legacy endpoint contract.
/// Constructing this strategy does not grant the sampling codec window support.
pub struct ResponsesLegacyCompactionStrategy {
    transport: Arc<ResponsesTransport>,
    binding: CompactionBinding,
    retry: LegacyCompactionRetryPolicy,
    accounting: Option<Arc<research_agent_core::context::accounting::ContextAccounting>>,
}
impl ResponsesLegacyCompactionStrategy {
    pub fn new(
        transport: Arc<ResponsesTransport>,
        binding: CompactionBinding,
    ) -> Result<Self, CompactionFailure> {
        binding.validate()?;
        if binding.kind != CompactionKind::RemoteWindow
            || binding.model.provider_id != transport.config.provider_id
        {
            return Err(CompactionFailure::Incompatible);
        }
        Ok(Self {
            transport,
            binding,
            retry: LegacyCompactionRetryPolicy::NONE,
            accounting: None,
        })
    }
    pub fn with_retry_policy(mut self, retry: LegacyCompactionRetryPolicy) -> Self {
        self.retry = retry;
        self
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
}
impl CompactionStrategy for ResponsesLegacyCompactionStrategy {
    fn input_accounting(
        &self,
    ) -> Option<Arc<research_agent_core::context::accounting::ContextAccounting>> {
        self.accounting.clone()
    }
    fn binding(&self) -> CompactionBinding {
        self.binding.clone()
    }
    fn retention_policy(
        &self,
    ) -> Option<research_agent_core::context::compaction_strategy::CompactionRetention> {
        Some(research_agent_core::context::compaction_strategy::CompactionRetention::ProviderWindow)
    }
    fn validate_output(&self, output: &CompactionOutput) -> Result<(), CompactionFailure> {
        validate_legacy_output(output)
    }
    fn compact(&self, request: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        Box::pin(async move {
            if request.binding != self.binding || request.source.model != self.binding.model {
                return Err(CompactionFailure::Incompatible);
            }
            let (body, account) = self
                .transport
                .compact_json(&request, self.retry, self.accounting.as_deref())
                .await?;
            if body
                .get("model")
                .is_some_and(|model| model.as_str().is_none_or(str::is_empty))
            {
                return Err(CompactionFailure::Incompatible);
            }
            let mut output = CompactionOutput::RemoteWindow(
                body["output"]
                    .as_array()
                    .cloned()
                    .ok_or(CompactionFailure::InvalidOutput)?,
            );
            self.validate_output(&output)?;
            if let CompactionOutput::RemoteWindow(items) = &mut output {
                items.retain(super::compaction_output::keep);
            }
            self.validate_output(&output)?;
            // Match V2: direct strategy callers also enforce the persisted
            // envelope budget, including the locally returned empty window.
            if serde_json::to_vec(&output)
                .map_err(|_| CompactionFailure::InvalidOutput)?
                .len()
                > request.max_result_bytes
            {
                return Err(CompactionFailure::OutputBudget);
            }
            let usage = match body.get("usage").filter(|value| !value.is_null()) {
                Some(usage) => ModelUsage {
                    input_tokens: Some(
                        usage["input_tokens"]
                            .as_u64()
                            .ok_or(CompactionFailure::InvalidOutput)?,
                    ),
                    output_tokens: Some(
                        usage["output_tokens"]
                            .as_u64()
                            .ok_or(CompactionFailure::InvalidOutput)?,
                    ),
                },
                None => ModelUsage::default(),
            };
            let response_id = match body.get("id").filter(|value| !value.is_null()) {
                Some(value) => Some(
                    value
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or(CompactionFailure::InvalidOutput)?
                        .to_owned(),
                ),
                None => None,
            };
            Ok(CompactionCandidate {
                binding: self.binding.clone(),
                credential_scope_id: Some(account),
                output,
                usage,
                response_id,
            })
        })
    }
}

impl ResponsesTransport {
    async fn compact_json(
        &self,
        request: &CompactionRequest,
        retry: LegacyCompactionRetryPolicy,
        accounting: Option<&research_agent_core::context::accounting::ContextAccounting>,
    ) -> Result<(Value, Uuid), ModelError> {
        let mut source =
            super::compaction_input::prepare_request(&request.source, self.capabilities().vision);
        source.request_id = request.operation_id;
        // Ordinary sampling forbids an empty source; legacy compact permits it.
        // Validate all other fields via a no-output assistant placeholder.
        let mut encoded_source = source.clone();
        if encoded_source.messages.is_empty() {
            encoded_source
                .messages
                .push(research_agent_core::model::ModelMessage::Assistant {
                    phase: None,
                    content: vec![],
                    tool_calls: vec![],
                });
        }
        let normal = self.request_body(&encoded_source, false)?;
        let mut body = serde_json::Map::new();
        for key in [
            "model",
            "input",
            "instructions",
            "tools",
            "parallel_tool_calls",
            "reasoning",
            "prompt_cache_key",
            "text",
        ] {
            if let Some(value) = normal.get(key) {
                body.insert(key.into(), value.clone());
            }
        }
        // Pinned CLI legacy API-key requests omit tier; V2 keeps its own policy.
        if !matches!(self.auth, Authentication::Platform(_))
            && let Some(tier) = normal.get("service_tier")
        {
            body.insert("service_tier".into(), tier.clone());
        }
        let mut lease = self.acquire_chatgpt_lease().await?;
        let account = lease
            .as_ref()
            .map_or(self.config.credential_scope_id, |lease| lease.scope_id());
        if account.is_nil() {
            return Err(ModelError::Authentication);
        }
        self.validate_native_history_scope(&source, account)?;
        let normalized = body["input"]
            .as_array()
            .cloned()
            .ok_or(ModelError::InvalidRequest)?;
        let mut body = Value::Object(body);
        let metadata = self
            .prepare_body_input(
                &mut body,
                &source,
                account,
                normalized.clone(),
                crate::agent::model::responses_native::capture_request(
                    &source,
                    normalized.len(),
                    false,
                    false,
                )?,
            )
            .await?;
        super::compaction_input::budget::trim_body(&mut body, &source, accounting)?;
        super::compaction_input::normalize_body(&mut body, self.capabilities().vision)?;
        // Check after expansion: no new messages can still mean installed history.
        if body["input"].as_array().is_some_and(Vec::is_empty) {
            if let Some(lease) = &lease {
                lease.check().map_err(|_| ModelError::Authentication)?;
            }
            return Ok((serde_json::json!({"output": []}), account));
        }
        // Fixed CompactionInput has no client_metadata field. Its window
        // identity is sent through compatibility headers only.
        body.as_object_mut()
            .ok_or(ModelError::InvalidRequest)?
            .remove("client_metadata");
        let bytes = serde_json::to_vec(&body).map_err(|_| ModelError::InvalidRequest)?;
        if bytes.len() > MAX_REQUEST {
            return Err(ModelError::BudgetExceeded);
        }
        let mut refreshed = false;
        let frozen = FrozenCompactHttpRequest {
            bytes,
            request_id: source.request_id,
            metadata,
        };
        let mut failures = 0;
        loop {
            match self
                .compact_json_attempt(
                    &frozen,
                    account,
                    &mut lease,
                    &mut refreshed,
                    request.max_result_bytes,
                )
                .await
            {
                Ok(value) => return Ok((value, account)),
                Err(error) => {
                    let Some(delay) = retry.delay(error, failures) else {
                        return Err(error);
                    };
                    failures += 1;
                    tokio::select! { biased;
                        _ = revoked(lease.as_deref()) => return Err(ModelError::Authentication),
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
            }
        }
    }

    async fn compact_json_attempt(
        &self,
        request: &FrozenCompactHttpRequest,
        account: Uuid,
        lease: &mut Option<Arc<ChatGptLease>>,
        refreshed: &mut bool,
        max_result_bytes: usize,
    ) -> Result<Value, ModelError> {
        let bytes = &request.bytes;
        let request_id = request.request_id;
        let mut response = loop {
            let mut http = self
                .client
                .post(format!("{}/compact", self.endpoint.trim_end_matches('/')))
                .header(ACCEPT, "application/json")
                .header(CONTENT_TYPE, "application/json")
                .header("X-Client-Request-Id", request_id.to_string())
                .timeout(self.config.timeout.saturating_mul(4))
                .body(bytes.to_vec())
                .build()
                .map_err(|_| ModelError::InvalidRequest)?;
            self.authorize(&mut http, lease.as_deref())?;
            if let Some(metadata) = &request.metadata {
                metadata.apply_headers(&mut http)?;
            }
            self.apply_turn_state(&mut http, account);
            let response = tokio::select! { biased;
                _ = revoked(lease.as_deref()) => return Err(ModelError::Authentication),
                response = self.execute(http) => response?,
            };
            if !*refreshed
                && self.can_refresh_chatgpt_lease()
                && matches!(response.status().as_u16(), 401 | 403)
            {
                let next = self
                    .acquire_chatgpt_lease()
                    .await?
                    .ok_or(ModelError::Authentication)?;
                if next.scope_id() != account {
                    return Err(ModelError::Authentication);
                }
                *lease = Some(next);
                *refreshed = true;
                continue;
            }
            validate_status(&response)?;
            break response;
        };
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .unwrap_or("")
            .trim();
        if !content_type.eq_ignore_ascii_case("application/json") {
            return Err(ModelError::InvalidStream);
        }
        // Guard streamed bytes even when Content-Length is absent or false.
        let limit = max_result_bytes.min(MAX_RESPONSE);
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err(ModelError::BudgetExceeded);
        }
        let mut bytes = Vec::new();
        loop {
            let chunk = tokio::select! { biased;
                _ = revoked(lease.as_deref()) => return Err(ModelError::Authentication),
                chunk = response.chunk() => chunk.map_err(crate::agent::model::transport_error::response)?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(ModelError::BudgetExceeded);
            }
            bytes.extend_from_slice(&chunk);
        }
        if let Some(lease) = lease {
            lease.check().map_err(|_| ModelError::Authentication)?;
        }
        self.capture_turn_state(&response, account);
        serde_json::from_slice(&bytes).map_err(|_| ModelError::InvalidStream)
    }
}
