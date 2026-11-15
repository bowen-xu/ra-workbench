//! Owned Chat Completions HTTP stream. No detached reader, retry, or reconnect.
use super::{
    chat_decoder::ChatDecoder,
    chat_request::{self, OutputTokenParameter},
    sse::SseDecoder,
};
use reqwest::{
    Client, Response,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue},
    redirect::Policy,
};
use research_agent_core::model::{
    ModelCapabilities, ModelError, ModelFuture, ModelRequest, ModelStream, ModelStreamEvent,
    ModelTransport,
};
use std::{collections::VecDeque, time::Duration};

pub struct ChatTransportConfig {
    pub request_policy: super::ChatRequestPolicy,
    pub provider_id: String,
    pub base_url: String,
    pub timeout: Duration,
    pub capabilities: ModelCapabilities,
    pub output_token_parameter: OutputTokenParameter,
}

pub struct ChatTransport {
    client: Client,
    config: ChatTransportConfig,
    endpoint: String,
    authorization: HeaderValue,
    developer_model: Option<research_agent_core::protocol::ModelIdentity>,
}

const MAX_EVENT_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_TOOL_CALLS: usize = 128;

impl ChatTransport {
    /// Native capability injection for one exact provider/model/revision.
    /// No URL/name inference and no system/user fallback for unsupported roles.
    pub fn with_developer_role(
        mut self,
        model: research_agent_core::protocol::ModelIdentity,
    ) -> Result<Self, ModelError> {
        if model.provider_id != self.config.provider_id || model.model_id.trim().is_empty() {
            return Err(ModelError::InvalidRequest);
        }
        self.developer_model = Some(model);
        Ok(self)
    }

    fn encode(&self, request: &ModelRequest) -> Result<serde_json::Value, ModelError> {
        if let Some(model) = &self.developer_model {
            if model != &request.model {
                return Err(ModelError::UnsupportedCapability);
            }
            chat_request::encode_with_developer_role(
                request,
                self.capabilities(),
                self.config.output_token_parameter,
                true,
            )
        } else {
            chat_request::encode(
                request,
                self.capabilities(),
                self.config.output_token_parameter,
            )
        }
    }
    /// Called only by native configuration/credential resolution. Header values
    /// are marked sensitive and neither config nor transport implements Debug.
    pub fn new(
        config: ChatTransportConfig,
        authorization: HeaderValue,
    ) -> Result<Self, ModelError> {
        crate::ai::validate_base_url(&config.provider_id, &config.base_url)
            .map_err(|_| ModelError::InvalidRequest)?;
        let endpoint = format!(
            "{}/chat/completions",
            config.base_url.trim().trim_end_matches('/')
        );
        Self::build(config, authorization, endpoint)
    }

    fn build(
        config: ChatTransportConfig,
        mut authorization: HeaderValue,
        endpoint: String,
    ) -> Result<Self, ModelError> {
        if config.timeout < Duration::from_secs(1) || config.timeout > Duration::from_secs(300) {
            return Err(ModelError::InvalidRequest);
        }
        if authorization.is_empty() {
            return Err(ModelError::Authentication);
        }
        authorization.set_sensitive(true);
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(config.timeout.min(Duration::from_secs(15)))
            .build()
            .map_err(|_| ModelError::Unavailable)?;
        Ok(Self {
            client,
            config,
            endpoint,
            authorization,
            developer_model: None,
        })
    }
}

impl ModelTransport for ChatTransport {
    fn capabilities(&self) -> ModelCapabilities {
        self.config.capabilities.clone()
    }
    fn validate_compaction_window<'a>(
        &'a self,
        checkpoint: &'a research_agent_core::context::checkpoint::ContextCheckpoint,
        request: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        Box::pin(async move {
            use research_agent_core::context::compaction_strategy::CompactionOutput;
            if !matches!(
                checkpoint.output,
                CompactionOutput::TextSummary(_) | CompactionOutput::ResetWindow
            ) || checkpoint.credential_scope_id.is_some()
            {
                return Err(ModelError::UnsupportedCapability);
            }
            checkpoint
                .validate(usize::MAX)
                .map_err(|_| ModelError::InvalidRequest)?;
            if request.model.provider_id != self.config.provider_id {
                return Err(ModelError::InvalidRequest);
            }
            let mut body = self.encode(request)?;
            self.config.request_policy.apply(request, &mut body)?;
            Ok(())
        })
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            if request.model.provider_id != self.config.provider_id {
                return Err(ModelError::InvalidRequest);
            }
            let mut body = self.encode(&request)?;
            self.config.request_policy.apply(&request, &mut body)?;
            let body = serde_json::to_vec(&body).map_err(|_| ModelError::InvalidRequest)?;
            if body.len() > MAX_REQUEST_BYTES {
                return Err(ModelError::BudgetExceeded);
            }
            let response = self
                .client
                .post(&self.endpoint)
                .header(AUTHORIZATION, self.authorization.clone())
                .header(ACCEPT, "text/event-stream")
                .header(CONTENT_TYPE, "application/json")
                .header("X-Client-Request-Id", request.request_id.to_string())
                .timeout(self.config.timeout)
                .body(body)
                .send()
                .await
                .map_err(super::transport_error::request)?;
            if !response.status().is_success() {
                return Err(match response.status().as_u16() {
                    status @ (401 | 403) => ModelError::HttpAuthentication(status),
                    status @ 429 => ModelError::HttpRateLimited(status),
                    status => ModelError::UnexpectedStatus(status),
                });
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .unwrap_or("")
                .trim();
            if !content_type.eq_ignore_ascii_case("text/event-stream") {
                return Err(ModelError::InvalidStream);
            }
            if response
                .content_length()
                .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
            {
                return Err(ModelError::BudgetExceeded);
            }
            Ok(Box::new(ChatHttpStream {
                response: Some(response),
                sse: SseDecoder::new(MAX_EVENT_BYTES),
                decoder: ChatDecoder::new(
                    self.config.provider_id.clone(),
                    MAX_TOOL_CALLS,
                    MAX_EVENT_BYTES,
                )?,
                events: VecDeque::new(),
                received: 0,
                failed: false,
            }) as Box<dyn ModelStream>)
        })
    }
}

struct ChatHttpStream {
    response: Option<Response>,
    sse: SseDecoder,
    decoder: ChatDecoder,
    events: VecDeque<ModelStreamEvent>,
    received: usize,
    failed: bool,
}
impl ChatHttpStream {
    async fn read_next(&mut self) -> Result<Option<ModelStreamEvent>, ModelError> {
        if self.failed {
            return Err(ModelError::InvalidStream);
        }
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            let Some(response) = self.response.as_mut() else {
                return Ok(None);
            };
            let bytes = response
                .chunk()
                .await
                .map_err(super::transport_error::response)?
                .ok_or(ModelError::IncompleteStream)?;
            self.received = self
                .received
                .checked_add(bytes.len())
                .ok_or(ModelError::BudgetExceeded)?;
            if self.received > MAX_RESPONSE_BYTES {
                return Err(ModelError::BudgetExceeded);
            }
            for data in self.sse.feed(&bytes)? {
                let events = self.decoder.decode(&data)?;
                let complete = events
                    .iter()
                    .any(|event| matches!(event, ModelStreamEvent::Completed { .. }));
                self.events.extend(events);
                if complete {
                    // Stop reading at the protocol terminator, even if the server
                    // keeps the HTTP body open. Dropping the response releases it.
                    self.response.take();
                    break;
                }
            }
        }
    }
}
impl ModelStream for ChatHttpStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move {
            let result = self.read_next().await;
            if result.is_err() {
                self.failed = true;
                self.events.clear();
                self.response.take();
            }
            result
        })
    }
}

#[cfg(test)]
#[path = "chat_transport/tests.rs"]
mod tests;
