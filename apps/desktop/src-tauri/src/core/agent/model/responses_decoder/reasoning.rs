//! Default fixed Codex turn.rs reasoning path: deltas + section additions.
//! Duplicate text.done belongs to its opt-in sequential feature and is ignored.
use super::*;
use research_agent_core::protocol::{ReasoningSummaryDraft, ReasoningSummaryIndex};

impl ResponsesDecoder {
    pub(super) fn start_summary(&mut self, id: &str) -> Result<Vec<ModelStreamEvent>, ModelError> {
        if self.summaries.contains_key(id) {
            return Ok(vec![]);
        }
        self.text_bytes = self
            .text_bytes
            .checked_add(id.len().saturating_add(1))
            .ok_or(ModelError::BudgetExceeded)?;
        self.summary_metadata_bytes = self
            .summary_metadata_bytes
            .checked_add(2)
            .ok_or(ModelError::BudgetExceeded)?;
        if self.text_bytes > self.max_bytes
            || self
                .summary_metadata_bytes
                .saturating_add(self.retained_bytes)
                > self.max_bytes
        {
            return Err(ModelError::BudgetExceeded);
        }
        self.summaries
            .insert(id.into(), ReasoningSummaryDraft::default());
        Ok(vec![ModelStreamEvent::ReasoningSummaryStarted {
            item_id: id.into(),
        }])
    }
    pub(super) fn summary_progress(
        &mut self,
        event: &Value,
        kind: &str,
    ) -> Result<Vec<ModelStreamEvent>, ModelError> {
        let Some((id, _)) = self
            .active_item
            .as_ref()
            .filter(|(_, kind)| kind == "reasoning")
        else {
            return Ok(vec![]);
        };
        let Some(index) = event["summary_index"]
            .as_i64()
            .map(ReasoningSummaryIndex::from)
        else {
            return Ok(vec![]);
        };
        let text = if kind == "response.reasoning_summary_text.delta" {
            let Some(text) = event["delta"].as_str() else {
                return Ok(vec![]);
            };
            Some(text)
        } else {
            None
        };
        let Some(draft) = self.summaries.get_mut(id) else {
            return Ok(vec![]);
        };
        let metadata = draft.metadata_growth(&index);
        self.summary_metadata_bytes = self
            .summary_metadata_bytes
            .checked_add(metadata)
            .ok_or(ModelError::BudgetExceeded)?;
        self.text_bytes = self
            .text_bytes
            .checked_add(text.map_or(0, str::len))
            .ok_or(ModelError::BudgetExceeded)?;
        if self.text_bytes > self.max_bytes
            || self
                .summary_metadata_bytes
                .saturating_add(self.retained_bytes)
                > self.max_bytes
        {
            return Err(ModelError::BudgetExceeded);
        }
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            draft.append(index.clone(), text);
            return Ok(vec![ModelStreamEvent::ReasoningSummaryDelta {
                item_id: id.clone(),
                summary_index: index,
                text: text.into(),
            }]);
        }
        if draft.contains(&index) {
            return Ok(vec![]);
        }
        draft.add_part(index.clone());
        Ok(vec![ModelStreamEvent::ReasoningSummaryPartAdded {
            item_id: id.clone(),
            summary_index: index,
        }])
    }
}

pub(super) fn buffered_bytes(event: &ModelStreamEvent) -> Result<usize, ModelError> {
    let value = match event {
        ModelStreamEvent::ReasoningSummaryStarted { item_id } => {
            serde_json::json!({"type":"reasoningSummaryStarted","itemId":item_id})
        }
        ModelStreamEvent::ReasoningSummaryPartAdded {
            item_id,
            summary_index,
        } => {
            serde_json::json!({"type":"reasoningSummaryPartAdded","itemId":item_id,"summaryIndex":summary_index})
        }
        ModelStreamEvent::ReasoningSummaryDelta {
            item_id,
            summary_index,
            text,
        } => {
            serde_json::json!({"type":"reasoningSummaryDelta","itemId":item_id,"summaryIndex":summary_index,"text":text})
        }
        _ => return Ok(0),
    };
    serde_json::to_vec(&value)
        .map(|bytes| bytes.len())
        .map_err(|_| ModelError::InvalidStream)
}
