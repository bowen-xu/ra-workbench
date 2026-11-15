//! V2 collector from fixed Codex compact_remote_v2.rs::collect_compaction_output.
//! Copyright 2025 OpenAI. Apache-2.0. Additional output is never executed/replayed.
use super::*;

impl ResponsesDecoder {
    pub(super) fn decode_compaction_event(
        &mut self,
        event: &Value,
        kind: &str,
    ) -> Result<Vec<ModelStreamEvent>, ModelError> {
        match kind {
            "response.output_item.done" => {
                let item = &event["item"];
                if !matches!(
                    item["type"].as_str(),
                    Some("compaction" | "compaction_summary")
                ) {
                    return Ok(vec![]);
                }
                if !item["encrypted_content"].is_string()
                    || item
                        .get("id")
                        .is_some_and(|id| !id.is_null() && !id.is_string())
                {
                    return Err(ModelError::InvalidStream);
                }
                self.retained_bytes = self
                    .retained_bytes
                    .checked_add(
                        serde_json::to_vec(item)
                            .map_err(|_| ModelError::InvalidStream)?
                            .len(),
                    )
                    .ok_or(ModelError::BudgetExceeded)?;
                if self.retained_bytes > self.max_bytes {
                    return Err(ModelError::BudgetExceeded);
                }
                // A repeated done item also counts as a second compaction. Do not
                // silently deduplicate it into a seemingly valid candidate.
                self.output.push(item.clone());
                Ok(vec![])
            }
            "response.completed" => {
                let response = &event["response"];
                string(response, "id")?;
                if self.output.len() != 1 {
                    return Err(ModelError::InvalidStream);
                }
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
                self.closed = true;
                self.completed = true;
                events.push(ModelStreamEvent::Completed {
                    reason: FinishReason::Stop,
                    end_turn: None,
                });
                Ok(events)
            }
            "response.incomplete" => Err(ModelError::IncompleteStream),
            "response.failed" | "error" => Err(failure::decode(event, kind)),
            // Mirrors the dedicated upstream collector: progress and unrelated
            // output do not populate model text, tool calls or replacement items.
            _ => Ok(vec![]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn completed_ignores_additive_status_but_still_requires_one_candidate() {
        let mut decoder = ResponsesDecoder::for_compaction("fixture".into(), 4096).unwrap();
        let item = json!({"type":"compaction","encrypted_content":"opaque"});
        decoder
            .decode(&json!({"type":"response.output_item.done","item":item}).to_string())
            .unwrap();
        let events = decoder.decode(&json!({"type":"response.completed","response":{"id":"r1","status":"future-status"}}).to_string()).unwrap();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ModelStreamEvent::Completed { .. }))
        );
        assert_eq!(decoder.take_completed_compaction().unwrap(), item);
        let mut empty = ResponsesDecoder::for_compaction("fixture".into(), 4096).unwrap();
        assert!(empty.decode(&json!({"type":"response.completed","response":{"id":"r2","status":"future-status"}}).to_string()).is_err());
    }

    #[test]
    fn completed_payload_cannot_substitute_for_a_missing_done_item() {
        let mut decoder = ResponsesDecoder::for_compaction("fixture".into(), 4096).unwrap();
        let item = json!({"type":"compaction","encrypted_content":"opaque"});
        let done = json!({"type":"response.output_item.done","item":item}).to_string();
        // A done candidate may precede the first observed response ID.
        assert!(decoder.decode(&done).unwrap().is_empty());
        let completed = json!({"type":"response.completed","response":{"id":"r1"}}).to_string();
        assert!(matches!(
            decoder.decode(&completed).unwrap().as_slice(),
            [
                ModelStreamEvent::StartedWithoutModel { .. },
                ModelStreamEvent::Completed { .. }
            ]
        ));
        assert_eq!(decoder.take_completed_compaction().unwrap(), item);
        // A terminal snapshot cannot replace the missing authoritative done.
        let mut decoder = ResponsesDecoder::for_compaction("fixture".into(), 4096).unwrap();
        decoder
            .decode(
                &json!({"type":"response.created","response":{"id":"r1","model":"fixture"}})
                    .to_string(),
            )
            .unwrap();
        assert!(matches!(
            decoder.take_completed_compaction(),
            Err(ModelError::IncompleteStream)
        ));
        assert!(matches!(
            decoder.decode(
                &json!({"type":"response.completed","response":{
            "id":"r1","model":"fixture","status":"completed","output":[item]}})
                .to_string()
            ),
            Err(ModelError::InvalidStream)
        ));
    }
}
