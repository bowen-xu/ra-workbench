//! Responses SSE mapping adapted from Codex codex-api/src/sse/responses.rs.
//! Output item identity, phase and admission order follow the fixed Codex stream.
// Copyright 2025 OpenAI. Adapted under Apache-2.0; local differences recorded in
// docs/architecture/shared-agent-codex-migration-map.md.
use super::responses_native::NativeScope;
use research_agent_core::{
    model::{FinishReason, ModelError, ModelStreamEvent, ModelUsage},
    protocol::{AssistantMessagePhase, ModelIdentity},
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Each instance owns one response. Completed common and native fields leave
/// together; opaque fields remain outside text and debug diagnostics.
pub struct ResponsesDecoder {
    provider: String,
    native_scope: Option<NativeScope>,
    response_id: Option<String>,
    actual_model: Option<String>,
    started: bool,
    buffered_events: Vec<ModelStreamEvent>,
    active_item: Option<(String, String)>,
    item_ids: HashSet<String>,
    call_ids: HashSet<String>,
    output: Vec<Value>,
    retained_bytes: usize,
    max_bytes: usize,
    max_calls: usize,
    closed: bool,
    completed: bool,
    text: HashMap<String, String>,
    text_bytes: usize,
    message_phases: HashMap<String, Option<AssistantMessagePhase>>,
    pending_items: HashMap<String, String>,
    compaction: bool,
    summaries: HashMap<String, research_agent_core::protocol::ReasoningSummaryDraft>,
    summary_metadata_bytes: usize,
    buffered_summary_bytes: usize,
}
impl ResponsesDecoder {
    pub(crate) fn new(
        scope: NativeScope,
        max_bytes: usize,
        max_calls: usize,
    ) -> Result<Self, ModelError> {
        if scope.account_scope.is_nil() {
            return Err(ModelError::Authentication);
        }
        Self::new_inner(
            scope.model.provider_id.clone(),
            max_bytes,
            max_calls,
            Some(scope),
        )
    }
    fn new_inner(
        provider: String,
        max_bytes: usize,
        max_calls: usize,
        native_scope: Option<NativeScope>,
    ) -> Result<Self, ModelError> {
        if provider.trim().is_empty() || max_bytes == 0 || max_calls == 0 {
            return Err(ModelError::InvalidRequest);
        }
        Ok(Self {
            provider,
            native_scope,
            response_id: None,
            actual_model: None,
            started: false,
            buffered_events: Vec::new(),
            active_item: None,
            item_ids: HashSet::new(),
            call_ids: HashSet::new(),
            output: Vec::new(),
            retained_bytes: 0,
            max_bytes,
            max_calls,
            closed: false,
            completed: false,
            text: HashMap::new(),
            text_bytes: 0,
            message_phases: HashMap::new(),
            pending_items: HashMap::new(),
            compaction: false,
            summaries: HashMap::new(),
            summary_metadata_bytes: 0,
            buffered_summary_bytes: 0,
        })
    }
    /// Seed the HTTP OpenAI-Model observation before SSE supplies a response ID.
    /// Like Codex spawn_response_stream, later SSE headers may update this model.
    pub fn with_server_model(mut self, model: String) -> Result<Self, ModelError> {
        if self.started || self.closed || model.trim().is_empty() {
            return Err(ModelError::InvalidStream);
        }
        if model.len() > self.max_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        self.actual_model = Some(model);
        Ok(self)
    }
    /// Dedicated V2 collector: additional output is protocol evidence only;
    /// none becomes assistant text or an executable tool call.
    pub fn for_compaction(provider: String, max_bytes: usize) -> Result<Self, ModelError> {
        let mut decoder = Self::new_inner(provider, max_bytes, 128, None)?;
        decoder.compaction = true;
        Ok(decoder)
    }
    pub fn take_completed_compaction(&mut self) -> Result<Value, ModelError> {
        if !self.compaction {
            return Err(ModelError::UnsupportedCapability);
        }
        let mut output = self.take_completed_output()?;
        let index = output
            .iter()
            .position(|item| {
                matches!(
                    item["type"].as_str(),
                    Some("compaction" | "compaction_summary")
                )
            })
            .ok_or(ModelError::InvalidStream)?;
        Ok(output.remove(index))
    }
    /// HTTP EOF is incomplete unless a Completed event has been returned.
    pub fn decode(&mut self, data: &str) -> Result<Vec<ModelStreamEvent>, ModelError> {
        if self.closed {
            return Err(ModelError::InvalidStream);
        }
        let result = self.decode_inner(data);
        if result.is_err() {
            self.closed = true;
            self.completed = false;
            self.output.clear();
            self.text.clear();
            self.message_phases.clear();
            self.pending_items.clear();
            self.buffered_events.clear();
            self.summaries.clear();
        }
        result
    }
    /// Only after successful protocol completion. This is provider wire state,
    /// not normalized conversation history; callers must bind it to the request.
    pub fn take_completed_output(&mut self) -> Result<Vec<Value>, ModelError> {
        if !self.completed {
            return Err(ModelError::IncompleteStream);
        }
        Ok(std::mem::take(&mut self.output))
    }
    fn decode_inner(&mut self, data: &str) -> Result<Vec<ModelStreamEvent>, ModelError> {
        if data.len() > self.max_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        let event: Value = serde_json::from_str(data).map_err(|_| ModelError::InvalidStream)?;
        let kind = string(&event, "type")?;
        let mut events = self.observe_identity(&event)?;
        let decoded = if self.compaction {
            self.decode_compaction_event(&event, kind)?
        } else {
            self.decode_response_event(&event, kind)?
        };
        if self.started {
            events.append(&mut self.buffered_events);
            self.buffered_summary_bytes = 0;
            events.extend(decoded);
            Ok(events)
        } else {
            // The fixed Codex fixture can reveal its response ID only at the
            // terminal. Item/text budgets also bound these pending events.
            for event in &decoded {
                self.buffered_summary_bytes = self
                    .buffered_summary_bytes
                    .checked_add(reasoning::buffered_bytes(event)?)
                    .ok_or(ModelError::BudgetExceeded)?;
            }
            if self.buffered_summary_bytes > self.max_bytes {
                return Err(ModelError::BudgetExceeded);
            }
            self.buffered_events.extend(decoded);
            Ok(vec![])
        }
    }
    fn decode_response_event(
        &mut self,
        event: &Value,
        kind: &str,
    ) -> Result<Vec<ModelStreamEvent>, ModelError> {
        match kind {
            "response.output_text.delta" | "response.refusal.delta" => {
                let item_id = self.message_delta_id(event)?;
                if self.item_ids.contains(&item_id) {
                    return Err(ModelError::InvalidStream);
                }
                let delta = event["delta"].as_str().ok_or(ModelError::InvalidStream)?;
                let mut events = self.start_message(&item_id, None)?;
                self.text_bytes = self
                    .text_bytes
                    .checked_add(delta.len())
                    .ok_or(ModelError::BudgetExceeded)?;
                if self.text_bytes > self.max_bytes {
                    return Err(ModelError::BudgetExceeded);
                }
                self.text
                    .entry(item_id.clone())
                    .or_default()
                    .push_str(delta);
                events.push(ModelStreamEvent::AssistantTextDelta {
                    item_id,
                    text: delta.to_owned(),
                });
                Ok(events)
            }
            "response.output_item.added" => self.item_added(&event["item"]),
            "response.output_item.done" => self.item_done(&event["item"]),
            "response.reasoning_summary_part.added" | "response.reasoning_summary_text.delta" => {
                self.summary_progress(event, kind)
            }
            "response.completed" => {
                let response = &event["response"];
                string(response, "id")?;
                if !self.pending_items.is_empty() || !self.text.is_empty() {
                    return Err(ModelError::InvalidStream);
                }
                // Like Codex ResponseCompleted, the event type is authoritative;
                // the additive status field is not part of completion decoding.
                // Codex admits output_item.done in arrival order. A terminal
                // snapshot is neither required nor a second authority that can
                // replace, reorder or authorize previously unobserved items.
                let mut events = Vec::new();
                if !response["usage"].is_null() {
                    let usage = &response["usage"];
                    events.push(ModelStreamEvent::Usage(ModelUsage {
                        input_tokens: Some(
                            usage["input_tokens"]
                                .as_u64()
                                .ok_or(ModelError::InvalidStream)?,
                        ),
                        output_tokens: Some(
                            usage["output_tokens"]
                                .as_u64()
                                .ok_or(ModelError::InvalidStream)?,
                        ),
                    }));
                }
                let end_turn = match response.get("end_turn") {
                    None | Some(Value::Null) => None,
                    Some(Value::Bool(value)) => Some(*value),
                    Some(_) => return Err(ModelError::InvalidStream),
                };
                self.closed = true;
                self.completed = true;
                events.push(ModelStreamEvent::Completed {
                    reason: if self.call_ids.is_empty() {
                        FinishReason::Stop
                    } else {
                        FinishReason::ToolCalls
                    },
                    end_turn,
                });
                Ok(events)
            }
            "response.incomplete" => Err(
                match event["response"]["incomplete_details"]["reason"].as_str() {
                    Some("max_output_tokens") => ModelError::Truncated,
                    Some("content_filter") => ModelError::Filtered,
                    _ => ModelError::IncompleteStream,
                },
            ),
            "response.failed" | "error" => Err(failure::decode(event, kind)),
            // These events carry progress or duplicate the authoritative done item.
            "response.in_progress"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.output_text.done"
            | "response.refusal.done"
            | "response.function_call_arguments.delta"
            | "response.function_call_arguments.done"
            | "response.reasoning_summary_part.done"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.delta"
            | "response.reasoning_text.done" => Ok(vec![]),
            // Codex process_responses_event ignores additive progress metadata.
            // Only a validated output_item.done grants item authority.
            _ => Ok(vec![]),
        }
    }
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ModelError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ModelError::InvalidStream)
}

#[cfg(test)]
#[path = "responses_decoder/tests.rs"]
mod tests;

#[path = "responses_decoder/compaction.rs"]
mod compaction;
#[path = "responses_decoder/failure.rs"]
mod failure;
#[path = "responses_decoder/identity.rs"]
mod identity;
#[path = "responses_decoder/items.rs"]
mod items;
#[path = "responses_decoder/messages.rs"]
mod messages;
#[path = "responses_decoder/reasoning.rs"]
mod reasoning;

pub(super) fn native_phase(item: &Value) -> Result<Option<AssistantMessagePhase>, ModelError> {
    messages::phase(item)
}
