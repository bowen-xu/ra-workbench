//! Item admission follows Codex turn.rs::assign_missing_streamed_response_item_id.
//! Copyright 2025 OpenAI. Adapted under Apache-2.0.
use super::*;

impl ResponsesDecoder {
    pub(super) fn item_added(&mut self, item: &Value) -> Result<Vec<ModelStreamEvent>, ModelError> {
        let kind = string(item, "type")?;
        let id = item_id(item)?.unwrap_or_else(|| local_item_id(kind));
        if self.item_ids.contains(&id) || self.pending_items.contains_key(&id) {
            return Err(ModelError::InvalidStream);
        }
        self.pending_items.insert(id.clone(), kind.to_owned());
        self.charge_item_bytes(id.len().saturating_add(kind.len()).saturating_add(1))?;
        self.active_item = Some((id.clone(), kind.to_owned()));
        if kind == "message" {
            if string(item, "role")? != "assistant" || self.message_phases.contains_key(&id) {
                return Err(ModelError::InvalidStream);
            }
            self.start_message(&id, messages::phase(item)?)
        } else if kind == "reasoning" {
            self.start_summary(&id)
        } else {
            Ok(vec![])
        }
    }

    pub(super) fn message_delta_id(&mut self, event: &Value) -> Result<String, ModelError> {
        let id = match event.get("item_id") {
            None | Some(Value::Null) => self
                .active_item
                .as_ref()
                .filter(|(_, kind)| kind == "message")
                .map(|(id, _)| id.clone())
                .ok_or(ModelError::InvalidStream)?,
            Some(Value::String(id)) if !id.is_empty() => id.clone(),
            Some(_) => return Err(ModelError::InvalidStream),
        };
        if self
            .pending_items
            .get(&id)
            .is_some_and(|kind| kind != "message")
        {
            return Err(ModelError::InvalidStream);
        }
        self.pending_items
            .entry(id.clone())
            .or_insert_with(|| "message".into());
        if self.active_item.is_none() {
            self.active_item = Some((id.clone(), "message".into()));
        }
        Ok(id)
    }

    pub(super) fn item_done(
        &mut self,
        original: &Value,
    ) -> Result<Vec<ModelStreamEvent>, ModelError> {
        let kind = string(original, "type")?;
        let id = match item_id(original)? {
            Some(id) => {
                if self
                    .active_item
                    .as_ref()
                    .is_some_and(|(active_id, active_kind)| {
                        active_kind == kind
                            && *active_id != id
                            && !self.pending_items.contains_key(&id)
                    })
                {
                    return Err(ModelError::InvalidStream);
                }
                id
            }
            None => match self.active_item.as_ref() {
                Some((id, active_kind)) if active_kind == kind => id.clone(),
                Some(_) => return Err(ModelError::InvalidStream),
                None => local_item_id(kind),
            },
        };
        if self
            .pending_items
            .get(&id)
            .is_some_and(|known_kind| known_kind != kind)
        {
            return Err(ModelError::InvalidStream);
        }
        if !self.item_ids.insert(id.clone()) {
            return Err(ModelError::InvalidStream);
        }
        let mut item = original.clone();
        item["id"] = Value::String(id.clone());
        self.pending_items.remove(&id);
        if self
            .active_item
            .as_ref()
            .is_some_and(|(active_id, _)| *active_id == id)
        {
            self.active_item = None;
        }
        let mut events = Vec::new();
        match kind {
            "function_call" => {
                if self.call_ids.len() >= self.max_calls {
                    return Err(ModelError::BudgetExceeded);
                }
                let call_id = string(&item, "call_id")?.to_owned();
                if !self.call_ids.insert(call_id.clone()) {
                    return Err(ModelError::InvalidStream);
                }
            }
            "message" => {
                events.extend(self.message_done(&item)?);
                self.restore_message_phase(&mut item);
            }
            "reasoning" => {
                if let Some(draft) = self.summaries.remove(&id) {
                    self.summary_metadata_bytes = self
                        .summary_metadata_bytes
                        .checked_sub(draft.metadata_bytes())
                        .ok_or(ModelError::InvalidStream)?;
                }
            }
            _ => return Err(ModelError::UnsupportedResponseType),
        }
        let scope = self
            .native_scope
            .as_ref()
            .ok_or(ModelError::InvalidStream)?;
        let completed = super::super::responses_native::capture(
            &item,
            scope,
            self.response_id.clone(),
            self.actual_model.as_ref().map(|model_id| ModelIdentity {
                provider_id: self.provider.clone(),
                model_id: model_id.clone(),
                revision: None,
            }),
        )?;
        // Count the admitted representation, including its metadata envelope,
        // before publishing any part of this done snapshot.
        self.charge_item_bytes(
            serde_json::to_vec(&completed)
                .map_err(|_| ModelError::InvalidStream)?
                .len(),
        )?;
        events.push(ModelStreamEvent::OutputItemDone(Box::new(completed)));
        self.output.push(item);
        Ok(events)
    }

    fn charge_item_bytes(&mut self, bytes: usize) -> Result<(), ModelError> {
        self.retained_bytes = self
            .retained_bytes
            .checked_add(bytes)
            .ok_or(ModelError::BudgetExceeded)?;
        if self.retained_bytes > self.max_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        Ok(())
    }
}

fn item_id(item: &Value) -> Result<Option<String>, ModelError> {
    match item.get("id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) if id.is_empty() => Ok(None),
        Some(Value::String(id)) => Ok(Some(id.clone())),
        Some(_) => Err(ModelError::InvalidStream),
    }
}

fn local_item_id(kind: &str) -> String {
    let prefix = match kind {
        "message" => "msg",
        "function_call" => "fc",
        "reasoning" => "rs",
        _ => "item",
    };
    format!("{prefix}_{}", uuid::Uuid::now_v7())
}
