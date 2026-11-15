//! Per-message boundaries adapted from fixed Codex output item handling.
//! Copyright 2025 OpenAI. Apache-2.0. Native IDs remain transport scoped.
use super::*;

pub(super) fn phase(item: &Value) -> Result<Option<AssistantMessagePhase>, ModelError> {
    match item.get("phase") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value == "commentary" => {
            Ok(Some(AssistantMessagePhase::Commentary))
        }
        Some(Value::String(value)) if value == "final_answer" => {
            Ok(Some(AssistantMessagePhase::FinalAnswer))
        }
        Some(_) => Err(ModelError::InvalidStream),
    }
}

impl ResponsesDecoder {
    pub(super) fn restore_message_phase(&self, item: &mut Value) {
        if item["type"].as_str() != Some("message") || !item["phase"].is_null() {
            return;
        }
        let Some(phase) = item["id"]
            .as_str()
            .and_then(|id| self.message_phases.get(id))
            .copied()
            .flatten()
        else {
            return;
        };
        let value = match phase {
            AssistantMessagePhase::Commentary => "commentary",
            AssistantMessagePhase::FinalAnswer => "final_answer",
            AssistantMessagePhase::Reasoning => return,
        };
        item["phase"] = Value::String(value.to_owned());
    }

    pub(super) fn start_message(
        &mut self,
        id: &str,
        phase: Option<AssistantMessagePhase>,
    ) -> Result<Vec<ModelStreamEvent>, ModelError> {
        if self.message_phases.contains_key(id) {
            return Ok(vec![]);
        }
        self.text_bytes = self
            .text_bytes
            .checked_add(id.len().saturating_add(1))
            .ok_or(ModelError::BudgetExceeded)?;
        if self.text_bytes > self.max_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        self.message_phases.insert(id.to_owned(), phase);
        Ok(vec![ModelStreamEvent::AssistantMessageStarted {
            item_id: id.to_owned(),
            phase,
        }])
    }

    pub(super) fn message_done(
        &mut self,
        item: &Value,
    ) -> Result<Vec<ModelStreamEvent>, ModelError> {
        if string(item, "role")? != "assistant" {
            return Err(ModelError::InvalidStream);
        }
        let id = string(item, "id")?;
        let done_phase = phase(item)?;
        let mut events = self.start_message(id, done_phase)?;
        let start_phase = self.message_phases.get(id).copied().flatten();
        self.message_phases
            .insert(id.to_owned(), done_phase.or(start_phase));
        let mut complete = String::new();
        for part in item["content"]
            .as_array()
            .ok_or(ModelError::InvalidStream)?
        {
            if let Some(key) = super::super::responses_native::content_text_field(part)? {
                complete.push_str(part[key].as_str().ok_or(ModelError::InvalidStream)?);
            }
        }
        let streamed = self.text.remove(id).unwrap_or_default();
        // Codex finalizes from output_item.done rather than requiring the
        // completed message to equal its streamed draft. Only append a missing
        // suffix here; a corrected body/phase travels atomically in the done
        // item, never as a delta concatenated onto the old draft.
        if let Some(suffix) = complete.strip_prefix(&streamed)
            && !suffix.is_empty()
        {
            events.push(ModelStreamEvent::AssistantTextDelta {
                item_id: id.to_owned(),
                text: suffix.to_owned(),
            });
        }
        Ok(events)
    }
}
