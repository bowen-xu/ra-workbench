//! Chat Completions SSE data payloads → shared model events.
//! Protocol basis: OpenAI function-calling and create-chat-completion docs.
//! SSE framing and HTTP ownership belong to the transport, not this decoder.
use research_agent_core::{
    model::{FinishReason, ModelError, ModelStreamEvent, ModelUsage},
    protocol::ModelIdentity,
};
use serde::Deserialize;
use std::collections::BTreeMap;

/// A request asks for exactly one choice. A malformed response permanently
/// poisons this decoder so its partially observed calls cannot be completed.
pub struct ChatDecoder {
    provider_id: String,
    identity: Option<(String, String)>,
    calls: BTreeMap<u32, (String, String)>,
    max_calls: usize,
    max_event_bytes: usize,
    finish: Option<FinishReason>,
    closed: bool,
}

impl ChatDecoder {
    pub fn new(
        provider_id: String,
        max_calls: usize,
        max_event_bytes: usize,
    ) -> Result<Self, ModelError> {
        if provider_id.trim().is_empty() || max_calls == 0 || max_event_bytes == 0 {
            return Err(ModelError::InvalidRequest);
        }
        Ok(Self {
            provider_id,
            identity: None,
            calls: BTreeMap::new(),
            max_calls,
            max_event_bytes,
            finish: None,
            closed: false,
        })
    }

    /// `[DONE]` is required even after finish_reason: a usage-only chunk may
    /// follow the last choice. EOF must be handled as IncompleteStream by HTTP.
    pub fn decode(&mut self, data: &str) -> Result<Vec<ModelStreamEvent>, ModelError> {
        if self.closed {
            return Err(ModelError::InvalidStream);
        }
        let result = self.decode_inner(data);
        if result.is_err() {
            self.closed = true;
        }
        result
    }

    fn decode_inner(&mut self, data: &str) -> Result<Vec<ModelStreamEvent>, ModelError> {
        if data.len() > self.max_event_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        if data == "[DONE]" {
            let finish = self.finish.ok_or(ModelError::IncompleteStream)?;
            self.closed = true;
            let mut events = Vec::new();
            // Truncated/filtered calls may contain incomplete JSON. The shared
            // assembler must receive the failure reason without parsing them.
            if matches!(finish, FinishReason::Stop | FinishReason::ToolCalls) {
                for (id, _) in self.calls.values() {
                    events.push(ModelStreamEvent::ToolCallCompleted {
                        call_id: id.clone(),
                    });
                }
            }
            events.push(ModelStreamEvent::Completed {
                reason: finish,
                end_turn: None,
            });
            return Ok(events);
        }
        let chunk: Chunk = serde_json::from_str(data).map_err(|_| ModelError::InvalidStream)?;
        if chunk.error.is_some()
            || chunk.object != "chat.completion.chunk"
            || chunk.id.is_empty()
            || chunk.model.trim().is_empty()
            || chunk.choices.len() > 1
        {
            return Err(ModelError::InvalidStream);
        }
        let mut events = Vec::new();
        match &self.identity {
            Some((id, model)) if id != &chunk.id || model != &chunk.model => {
                return Err(ModelError::InvalidStream);
            }
            None => {
                self.identity = Some((chunk.id.clone(), chunk.model.clone()));
                events.push(ModelStreamEvent::Started {
                    response_id: chunk.id,
                    actual_model: ModelIdentity {
                        provider_id: self.provider_id.clone(),
                        model_id: chunk.model,
                        revision: None,
                    },
                });
            }
            _ => {}
        }
        if let Some(choice) = chunk.choices.into_iter().next() {
            if choice.index != 0 || self.finish.is_some() {
                return Err(ModelError::InvalidStream);
            }
            let delta = choice.delta;
            if delta
                .role
                .as_deref()
                .is_some_and(|role| role != "assistant")
                || delta.function_call.is_some()
                || delta.audio.is_some()
            {
                return Err(ModelError::InvalidStream);
            }
            // Refusals are visible assistant output, not silently discarded.
            for text in [delta.content, delta.refusal].into_iter().flatten() {
                if !text.is_empty() {
                    events.push(ModelStreamEvent::TextDelta(text));
                }
            }
            for call in delta.tool_calls.unwrap_or_default() {
                self.tool_delta(call, &mut events)?;
            }
            if let Some(reason) = choice.finish_reason {
                let finish = match reason.as_str() {
                    "stop" => FinishReason::Stop,
                    "tool_calls" => FinishReason::ToolCalls,
                    "length" => FinishReason::Length,
                    "content_filter" => FinishReason::Filtered,
                    _ => return Err(ModelError::InvalidStream),
                };
                if (finish == FinishReason::Stop && !self.calls.is_empty())
                    || (finish == FinishReason::ToolCalls && self.calls.is_empty())
                {
                    return Err(ModelError::InvalidStream);
                }
                self.finish = Some(finish);
            }
        }
        if let Some(usage) = chunk.usage {
            events.push(ModelStreamEvent::Usage(ModelUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
            }));
        }
        Ok(events)
    }

    fn tool_delta(
        &mut self,
        call: CallDelta,
        events: &mut Vec<ModelStreamEvent>,
    ) -> Result<(), ModelError> {
        if call.kind.as_deref().is_some_and(|kind| kind != "function") {
            return Err(ModelError::InvalidStream);
        }
        let function = call.function.ok_or(ModelError::InvalidStream)?;
        if let Some((id, name)) = self.calls.get(&call.index) {
            // Qwen continuation chunks use an empty ID after establishing the call.
            if call
                .id
                .as_ref()
                .filter(|value| !value.is_empty())
                .is_some_and(|value| value != id)
                || function.name.as_ref().is_some_and(|value| value != name)
            {
                return Err(ModelError::InvalidStream);
            }
        } else {
            if self.calls.len() >= self.max_calls {
                return Err(ModelError::BudgetExceeded);
            }
            let id = call.id.ok_or(ModelError::InvalidStream)?;
            let name = function.name.ok_or(ModelError::InvalidStream)?;
            if id.is_empty()
                || name.trim().is_empty()
                || self.calls.values().any(|(existing, _)| existing == &id)
            {
                return Err(ModelError::InvalidStream);
            }
            events.push(ModelStreamEvent::ToolCallStarted {
                call_id: id.clone(),
                name: name.clone(),
            });
            self.calls.insert(call.index, (id, name));
        }
        if let Some(delta) = function.arguments.filter(|value| !value.is_empty()) {
            events.push(ModelStreamEvent::ToolArgumentsDelta {
                call_id: self.calls[&call.index].0.clone(),
                delta,
            });
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Chunk {
    error: Option<serde_json::Value>,
    id: String,
    object: String,
    model: String,
    choices: Vec<Choice>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Choice {
    index: u32,
    delta: Delta,
    finish_reason: Option<String>,
}
#[derive(Default, Deserialize)]
struct Delta {
    role: Option<String>,
    content: Option<String>,
    refusal: Option<String>,
    tool_calls: Option<Vec<CallDelta>>,
    function_call: Option<serde_json::Value>,
    audio: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct CallDelta {
    index: u32,
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<FunctionDelta>,
}
#[derive(Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}
#[derive(Deserialize)]
struct Usage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

#[cfg(test)]
#[path = "chat_decoder/tests.rs"]
mod tests;
