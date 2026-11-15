//! Owned Responses HTTP transport; no detached reads or automatic paid retries.
use super::responses_metadata::{PreparedResponsesInput, ResponsesInputKind};
use super::{
    responses_decoder::ResponsesDecoder,
    responses_request::{
        ResponsesEndpointKind, encode_responses_compaction_request, encode_responses_request,
    },
    sse::SseDecoder,
};
use crate::agent::authentication::{ChatGptCredentialSource, ChatGptLease};
use reqwest::{
    Client, Response,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue},
    redirect::Policy,
};
use research_agent_core::{
    model::{
        ModelCapabilities, ModelError, ModelFuture, ModelRequest, ModelStream, ModelStreamEvent,
        ModelTransport,
    },
    protocol::ModelIdentity,
};
use serde_json::Value;
use std::{collections::VecDeque, sync::Arc, time::Duration};
use uuid::Uuid;

const MAX_EVENT: usize = 1024 * 1024;
const MAX_RESPONSE: usize = 16 * 1024 * 1024;
const MAX_REQUEST: usize = 32 * 1024 * 1024;

/// Provider-owned completion, including encrypted reasoning. Not UI history.
/// Consumers must bind storage/replay to request/model/context and account scope.
pub struct ResponsesReceipt {
    pub request_id: Uuid,
    pub credential_scope_id: Uuid,
    pub requested_model: ModelIdentity,
    pub model: ModelIdentity,
    pub normalized_input: Vec<Value>,
    pub authority_items: Option<usize>,
    /// Expanded continuation prefix before prompt-only media normalization.
    /// This proves replay against the installed window; it is not HTTP evidence.
    pub input: Vec<Value>,
    pub output: Vec<Value>,
}
#[path = "responses_transport/receipts.rs"]
mod receipts;
pub use receipts::ResponsesReceiptSink;

pub struct ResponsesTransportConfig {
    /// Native API-key configuration revision. ChatGPT uses the lease epoch.
    pub credential_scope_id: Uuid,
    pub provider_id: String,
    pub timeout: Duration,
    pub service_tier: Option<String>,
    pub capabilities: ModelCapabilities,
}
enum Authentication {
    Platform(HeaderValue),
    ChatGpt(Arc<dyn ChatGptCredentialSource>),
    #[cfg(all(test, debug_assertions, feature = "development-data"))]
    Preview,
    #[cfg(test)]
    SyntheticLease(Arc<ChatGptLease>),
    #[cfg(test)]
    SyntheticChatGpt(Arc<dyn ChatGptCredentialSource>),
}
pub struct ResponsesTransport {
    #[cfg(all(test, debug_assertions, feature = "development-data"))]
    preview_wire: Option<Arc<dyn PreviewResponsesWire>>,
    client: Client,
    endpoint: String,
    kind: ResponsesEndpointKind,
    pub(super) config: ResponsesTransportConfig,
    pub(super) reasoning_summary: Option<(
        ModelIdentity,
        super::responses_reasoning::ResponsesReasoningSummary,
    )>,
    auth: Authentication,
    receipts: Arc<dyn ResponsesReceiptSink>,
    turn_state: std::sync::OnceLock<(Uuid, HeaderValue)>,
}
impl ResponsesTransport {
    /// Only the HTTP side effect is replaced. Body admission, state preparation,
    /// SSE decoding and compaction keep their production owners and lifetimes.
    #[cfg(all(test, debug_assertions, feature = "development-data"))]
    pub(crate) fn preview(
        config: ResponsesTransportConfig,
        wire: Arc<dyn PreviewResponsesWire>,
        receipts: Arc<dyn ResponsesReceiptSink>,
    ) -> Result<Self, ModelError> {
        if config.provider_id != "openai" || config.credential_scope_id.is_nil() {
            return Err(ModelError::InvalidRequest);
        }
        let mut transport = Self::build(
            config,
            "https://agent-preview.invalid/responses".into(),
            ResponsesEndpointKind::ChatGpt,
            Authentication::Preview,
            receipts,
        )?;
        transport.preview_wire = Some(wire);
        Ok(transport)
    }
    pub fn platform(
        config: ResponsesTransportConfig,
        base_url: &str,
        mut authorization: HeaderValue,
        receipts: Arc<dyn ResponsesReceiptSink>,
    ) -> Result<Self, ModelError> {
        crate::ai::validate_base_url(&config.provider_id, base_url)
            .map_err(|_| ModelError::InvalidRequest)?;
        if authorization.is_empty() {
            return Err(ModelError::Authentication);
        }
        if config.credential_scope_id.is_nil() {
            return Err(ModelError::InvalidRequest);
        }
        authorization.set_sensitive(true);
        Self::build(
            config,
            format!("{}/responses", base_url.trim().trim_end_matches('/')),
            ResponsesEndpointKind::Platform,
            Authentication::Platform(authorization),
            receipts,
        )
    }
    pub fn chatgpt(
        config: ResponsesTransportConfig,
        auth: Arc<dyn ChatGptCredentialSource>,
        receipts: Arc<dyn ResponsesReceiptSink>,
    ) -> Result<Self, ModelError> {
        if config.provider_id != "openai" {
            return Err(ModelError::InvalidRequest);
        }
        Self::build(
            config,
            "https://chatgpt.com/backend-api/codex/responses".into(),
            ResponsesEndpointKind::ChatGpt,
            Authentication::ChatGpt(auth),
            receipts,
        )
    }
    #[cfg(test)]
    pub(crate) fn synthetic_platform(
        config: ResponsesTransportConfig,
        base_url: &str,
        receipts: Arc<dyn ResponsesReceiptSink>,
    ) -> Result<Self, ModelError> {
        Self::build(
            config,
            format!("{base_url}/responses"),
            ResponsesEndpointKind::Platform,
            Authentication::Platform(HeaderValue::from_static("Bearer synthetic")),
            receipts,
        )
    }
    #[cfg(test)]
    pub(crate) fn use_synthetic_lease(&mut self, lease: ChatGptLease) {
        self.auth = Authentication::SyntheticLease(Arc::new(lease));
    }
    fn build(
        config: ResponsesTransportConfig,
        endpoint: String,
        kind: ResponsesEndpointKind,
        auth: Authentication,
        receipts: Arc<dyn ResponsesReceiptSink>,
    ) -> Result<Self, ModelError> {
        if config.service_tier.as_deref().is_some_and(|tier| {
            config.provider_id != "openai" || !matches!(tier, "fast" | "default")
        }) {
            return Err(ModelError::InvalidRequest);
        }
        if config.timeout < Duration::from_secs(1) || config.timeout > Duration::from_secs(300) {
            return Err(ModelError::InvalidRequest);
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(config.timeout.min(Duration::from_secs(15)))
            .build()
            .map_err(|_| ModelError::Unavailable)?;
        Ok(Self {
            #[cfg(all(test, debug_assertions, feature = "development-data"))]
            preview_wire: None,
            client,
            endpoint,
            kind,
            config,
            reasoning_summary: None,
            auth,
            receipts,
            turn_state: std::sync::OnceLock::new(),
        })
    }

    async fn acquire_chatgpt_lease(&self) -> Result<Option<Arc<ChatGptLease>>, ModelError> {
        match &self.auth {
            Authentication::Platform(_) => Ok(None),
            #[cfg(all(test, debug_assertions, feature = "development-data"))]
            Authentication::Preview => Ok(None),
            Authentication::ChatGpt(service) => service
                .acquire()
                .await
                .map(Arc::new)
                .map(Some)
                .map_err(|_| ModelError::Authentication),
            #[cfg(test)]
            Authentication::SyntheticLease(lease) => Ok(Some(lease.clone())),
            #[cfg(test)]
            Authentication::SyntheticChatGpt(service) => service
                .acquire()
                .await
                .map(Arc::new)
                .map(Some)
                .map_err(|_| ModelError::Authentication),
        }
    }

    fn can_refresh_chatgpt_lease(&self) -> bool {
        match self.auth {
            Authentication::ChatGpt(_) => true,
            #[cfg(test)]
            Authentication::SyntheticChatGpt(_) => true,
            _ => false,
        }
    }

    fn authorize(
        &self,
        request: &mut reqwest::Request,
        lease: Option<&ChatGptLease>,
    ) -> Result<(), ModelError> {
        match &self.auth {
            Authentication::Platform(header) => {
                request.headers_mut().insert(AUTHORIZATION, header.clone());
            }
            Authentication::ChatGpt(_) => lease
                .ok_or(ModelError::Authentication)?
                .authorize(request)
                .map_err(|_| ModelError::Authentication)?,
            #[cfg(all(test, debug_assertions, feature = "development-data"))]
            Authentication::Preview => {
                if self.preview_wire.is_none() {
                    return Err(ModelError::InvalidRequest);
                }
            }
            #[cfg(test)]
            Authentication::SyntheticLease(_) | Authentication::SyntheticChatGpt(_) => {}
        }
        Ok(())
    }
}
impl ModelTransport for ResponsesTransport {
    fn supports_compaction_kind(
        &self,
        _kind: research_agent_core::context::compaction_strategy::CompactionKind,
    ) -> bool {
        true
    }
    fn validate_compaction_window<'a>(
        &'a self,
        checkpoint: &'a research_agent_core::context::checkpoint::ContextCheckpoint,
        request: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        self.validate_installed_window(checkpoint, request)
    }
    fn capabilities(&self) -> ModelCapabilities {
        self.config.capabilities.clone()
    }
    fn supports_namespace_tools(&self) -> bool {
        matches!(self.kind, ResponsesEndpointKind::ChatGpt)
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            Ok(Box::new(self.start_owned(request, false).await?) as Box<dyn ModelStream>)
        })
    }
    fn start_text_compaction(
        &self,
        request: ModelRequest,
    ) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            Ok(
                Box::new(self.start_owned_inner(request, false, true, None).await?)
                    as Box<dyn ModelStream>,
            )
        })
    }
}
impl ResponsesTransport {
    fn validate_native_history_scope(
        &self,
        request: &ModelRequest,
        account: Uuid,
    ) -> Result<(), ModelError> {
        if matches!(self.kind, ResponsesEndpointKind::ChatGpt) {
            super::responses_native::validate_chatgpt_scope(request, account)
        } else {
            super::responses_native::validate_scope(request, account)
        }
    }
    fn request_body(&self, request: &ModelRequest, compaction: bool) -> Result<Value, ModelError> {
        if request.model.provider_id != self.config.provider_id {
            return Err(ModelError::InvalidRequest);
        }
        if !self.supports_namespace_tools()
            && request.tools.iter().any(|tool| tool.namespace.is_some())
        {
            return Err(ModelError::UnsupportedCapability);
        }
        let mut body = if compaction {
            let prepared = compaction_input::prepare_request(request, self.capabilities().vision);
            encode_responses_compaction_request(&prepared, self.capabilities(), self.kind)?
        } else {
            encode_responses_request(request, self.capabilities(), self.kind)?
        };
        if let Some(tier) = &self.config.service_tier {
            body["service_tier"] = Value::String(
                if tier == "fast" {
                    "priority"
                } else {
                    "default"
                }
                .into(),
            );
        }
        self.apply_reasoning_summary(&request.model, &mut body)?;
        Ok(body)
    }
    fn start_owned(
        &self,
        request: ModelRequest,
        compaction: bool,
    ) -> ModelFuture<'_, ResponsesHttpStream> {
        self.start_owned_inner(request, compaction, false, None)
    }
    fn start_owned_inner(
        &self,
        request: ModelRequest,
        compaction: bool,
        text_compaction: bool,
        accounting: Option<Arc<research_agent_core::context::accounting::ContextAccounting>>,
    ) -> ModelFuture<'_, ResponsesHttpStream> {
        Box::pin(async move {
            let mut body = self.request_body(&request, compaction)?;
            let mut lease = self.acquire_chatgpt_lease().await?;
            let credential_scope_id = lease
                .as_ref()
                .map_or(self.config.credential_scope_id, |lease| lease.scope_id());
            if credential_scope_id.is_nil() {
                return Err(ModelError::Authentication);
            }
            self.validate_native_history_scope(&request, credential_scope_id)?;
            let normalized_input = body["input"]
                .as_array()
                .cloned()
                .ok_or(ModelError::InvalidRequest)?;
            let captured = super::responses_native::capture_request(
                &request,
                normalized_input
                    .len()
                    .checked_sub(usize::from(compaction))
                    .ok_or(ModelError::InvalidRequest)?,
                text_compaction,
                !compaction && !text_compaction,
            )?;
            let metadata = self
                .prepare_body_input(
                    &mut body,
                    &request,
                    credential_scope_id,
                    normalized_input.clone(),
                    captured,
                )
                .await?;
            if compaction {
                compaction_input::budget::trim_body(&mut body, &request, accounting.as_deref())?;
                compaction_input::normalize_body(&mut body, self.capabilities().vision)?;
            } else {
                compaction_input::normalize_restored_media(&mut body, self.capabilities().vision)?;
            }
            let bytes = serde_json::to_vec(&body).map_err(|_| ModelError::InvalidRequest)?;
            if bytes.len() > MAX_REQUEST {
                return Err(ModelError::BudgetExceeded);
            }
            let mut retried_after_unauthorized = false;
            let response = loop {
                let mut http = self
                    .client
                    .post(&self.endpoint)
                    .header(ACCEPT, "text/event-stream")
                    .header(CONTENT_TYPE, "application/json")
                    .header("X-Client-Request-Id", request.request_id.to_string())
                    .timeout(self.config.timeout)
                    .body(bytes.clone())
                    .build()
                    .map_err(|_| ModelError::InvalidRequest)?;
                self.authorize(&mut http, lease.as_deref())?;
                if let Some(metadata) = &metadata {
                    metadata.apply_headers(&mut http)?;
                }
                self.apply_turn_state(&mut http, credential_scope_id);
                let response = tokio::select! {biased;
                    _=revoked(lease.as_deref())=>return Err(ModelError::Authentication),
                    response=self.execute(http)=>response?,
                };
                if !retried_after_unauthorized
                    && self.can_refresh_chatgpt_lease()
                    && matches!(response.status().as_u16(), 401 | 403)
                {
                    // Adapted from Apache-2.0 OpenAI Codex
                    // `core/src/client.rs::handle_unauthorized`: an explicitly
                    // rejected ChatGPT credential may be refreshed and retried
                    // once, before any response stream has been consumed.
                    let refreshed = self
                        .acquire_chatgpt_lease()
                        .await?
                        .ok_or(ModelError::Authentication)?;
                    if refreshed.scope_id() != credential_scope_id {
                        return Err(ModelError::Authentication);
                    }
                    lease = Some(refreshed);
                    retried_after_unauthorized = true;
                    continue;
                }
                validate_response(&response, self.kind)?;
                self.capture_turn_state(&response, credential_scope_id);
                break response;
            };
            let decoder = if compaction {
                ResponsesDecoder::for_compaction(self.config.provider_id.clone(), MAX_RESPONSE)?
            } else {
                ResponsesDecoder::new(
                    super::responses_native::NativeScope {
                        model: request.model.clone(),
                        account_scope: credential_scope_id,
                    },
                    MAX_RESPONSE,
                    128,
                )?
            };
            let decoder = match response
                .headers()
                .get("openai-model")
                .and_then(|value| value.to_str().ok())
            {
                Some(model) => decoder.with_server_model(model.to_owned())?,
                None => decoder,
            };
            Ok(ResponsesHttpStream {
                compaction,
                compaction_output: None,
                credential_scope_id,
                response: Some(response),
                lease,
                sse: SseDecoder::new(MAX_EVENT),
                decoder,
                events: VecDeque::new(),
                pending_data: VecDeque::new(),
                pending_error: None,
                received: 0,
                failed: false,
            })
        })
    }
}
#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) trait PreviewResponsesWire: Send + Sync {
    fn respond(&self, request: reqwest::Request) -> ModelFuture<'_, Response>;
}
impl ResponsesTransport {
    async fn execute(&self, request: reqwest::Request) -> Result<Response, ModelError> {
        #[cfg(all(test, debug_assertions, feature = "development-data"))]
        if let Some(wire) = &self.preview_wire {
            return wire.respond(request).await;
        }
        #[cfg(all(test, debug_assertions, feature = "development-data"))]
        if matches!(self.auth, Authentication::Preview) {
            return Err(ModelError::InvalidRequest);
        }
        self.client
            .execute(request)
            .await
            .map_err(super::transport_error::request)
    }
}
async fn revoked(lease: Option<&ChatGptLease>) {
    match lease {
        Some(lease) => lease.revoked().await,
        None => std::future::pending().await,
    }
}
fn validate_response(response: &Response, kind: ResponsesEndpointKind) -> Result<(), ModelError> {
    validate_status(response)?;
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .unwrap_or("")
        .trim();
    // Managed ChatGPT can omit Content-Type; explicit wrong types still fail.
    // SSE framing and terminal protocol validation remain mandatory.
    let omitted_chatgpt_type = matches!(kind, ResponsesEndpointKind::ChatGpt)
        && !response.headers().contains_key(CONTENT_TYPE);
    if !omitted_chatgpt_type && !content_type.eq_ignore_ascii_case("text/event-stream") {
        return Err(ModelError::InvalidStream);
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE as u64)
    {
        return Err(ModelError::BudgetExceeded);
    }
    Ok(())
}
fn validate_status(response: &Response) -> Result<(), ModelError> {
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            status @ (401 | 403) => ModelError::HttpAuthentication(status),
            status @ 429 => ModelError::HttpRateLimited(status),
            status => ModelError::UnexpectedStatus(status),
        });
    }
    Ok(())
}
struct ResponsesHttpStream {
    compaction: bool,
    compaction_output: Option<Value>,
    credential_scope_id: Uuid,
    response: Option<Response>,
    lease: Option<Arc<ChatGptLease>>,
    sse: SseDecoder,
    decoder: ResponsesDecoder,
    events: VecDeque<ModelStreamEvent>,
    pending_data: VecDeque<String>,
    pending_error: Option<ModelError>,
    received: usize,
    failed: bool,
}
impl ResponsesHttpStream {
    async fn read_next(&mut self) -> Result<Option<ModelStreamEvent>, ModelError> {
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            if let Some(data) = self.pending_data.pop_front() {
                self.decode_data(&data).await?;
                continue;
            }
            if let Some(error) = self.pending_error.take() {
                return Err(error);
            }
            let Some(response) = self.response.as_mut() else {
                return Ok(None);
            };
            let bytes = response
                .chunk()
                .await
                .map_err(super::transport_error::response)?
                .ok_or(ModelError::IncompleteStream)?;
            let accepted = bytes.len().min(MAX_RESPONSE.saturating_sub(self.received));
            self.received += accepted;
            let (frames, error) = self.sse.feed_partial(&bytes[..accepted]);
            self.pending_data.extend(frames);
            self.pending_error =
                error.or_else(|| (accepted < bytes.len()).then_some(ModelError::BudgetExceeded));
        }
    }

    async fn decode_data(&mut self, data: &str) -> Result<(), ModelError> {
        let events = self.decoder.decode(data)?;
        let completed = events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::Completed { .. }));
        if completed {
            self.response.take();
            self.pending_data.clear();
            self.pending_error = None;
            if self.compaction {
                self.compaction_output = Some(self.decoder.take_completed_compaction()?);
            } else {
                self.decoder.take_completed_output()?;
            }
        }
        self.events.extend(events);
        Ok(())
    }
}
impl ModelStream for ResponsesHttpStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move {
            if self.failed {
                return Err(ModelError::InvalidStream);
            }
            // Keep ownership in the stream even if this next() future is dropped.
            let lease = self.lease.clone();
            let result = tokio::select! {biased;
                _=revoked(lease.as_deref())=>Err(ModelError::Authentication),
                result=self.read_next()=>result,
            };
            if result.is_err() {
                self.failed = true;
                self.events.clear();
                self.pending_data.clear();
                self.pending_error = None;
                self.response.take();
            }
            result
        })
    }
}

#[cfg(test)]
#[path = "responses_transport/tests.rs"]
mod tests;

#[path = "responses_compaction.rs"]
mod compaction;
pub use compaction::ResponsesV2CompactionStrategy;
pub(crate) use compaction::retained_v2_inputs;
pub(crate) use compaction::validate_v2_output;

#[path = "responses_legacy_compaction.rs"]
mod legacy_compaction;
pub use legacy_compaction::ResponsesLegacyCompactionStrategy;
pub(crate) use legacy_compaction::validate_legacy_output;

#[path = "responses_turn_state.rs"]
mod turn_state;

#[path = "responses_legacy_retry.rs"]
mod legacy_retry;
pub use legacy_retry::LegacyCompactionRetryPolicy;

#[path = "responses_transport/compaction_input.rs"]
mod compaction_input;
#[path = "responses_transport/compaction_output.rs"]
mod compaction_output;
#[path = "responses_transport/metadata.rs"]
mod metadata;

pub(crate) use compaction_output::canonical_position as legacy_canonical_position;
