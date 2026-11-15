//! Installed Responses state remains provider data, not normalized user messages.
//! The durable window owner supplies this base inside its transaction.
use super::*;

impl ResponsesContinuation {
    pub(crate) fn bind_compaction_sync(
        &self,
        account: Uuid,
        model: &ModelIdentity,
        item: &Value,
        retained_inputs: usize,
    ) -> Result<(), ModelError> {
        if model != &self.model {
            return Err(ModelError::UnsupportedCapability);
        }
        if !matches!(
            item["type"].as_str(),
            Some("compaction" | "compaction_summary")
        ) || !item["encrypted_content"].is_string()
            || item
                .get("id")
                .is_some_and(|id| !id.is_null() && !id.is_string())
        {
            return Err(ModelError::UnsupportedCapability);
        }
        self.budget(item)?;
        self.check_credentials(account, true)?;
        let state = self.state.lock().map_err(|_| ModelError::Unavailable)?;
        if let Some(state) = state.as_ref() {
            if state.actual_model.provider_id != self.model.provider_id {
                return Err(ModelError::UnsupportedCapability);
            }
            let position = authority_boundary(&state.normalized_prefix, state.authority_items)?
                .checked_add(retained_inputs)
                .ok_or(ModelError::BudgetExceeded)?;
            if state.wire_prefix.get(position) != Some(item) {
                return Err(ModelError::InvalidStream);
            }
        }
        *self
            .compaction
            .lock()
            .map_err(|_| ModelError::Unavailable)? = Some(InstalledCompaction::Opaque {
            item: item.clone(),
            retained_inputs,
        });
        Ok(())
    }

    /// Bind the validated replacement, including a legitimate empty history.
    /// Keep its items unchanged; supplier filtering belongs to candidate creation.
    #[cfg(test)]
    pub(crate) fn bind_window_sync(
        &self,
        account: Uuid,
        model: &ModelIdentity,
        items: &[Value],
    ) -> Result<(), ModelError> {
        self.bind_window_with_canonical_sync(account, model, items, 0)
    }

    pub(crate) fn bind_window_with_canonical_sync(
        &self,
        account: Uuid,
        model: &ModelIdentity,
        items: &[Value],
        canonical_inputs: usize,
    ) -> Result<(), ModelError> {
        if model != &self.model {
            return Err(ModelError::UnsupportedCapability);
        }
        if items
            .iter()
            .any(|item| item.as_object().is_none_or(|value| value.is_empty()))
        {
            return Err(ModelError::InvalidStream);
        }
        self.budget(&items)?;
        self.check_credentials(account, true)?;
        let state = self.state.lock().map_err(|_| ModelError::Unavailable)?;
        if let Some(state) = state.as_ref() {
            if state.actual_model.provider_id != self.model.provider_id
                || state.actual_model.model_id != self.model.model_id
            {
                return Err(ModelError::UnsupportedCapability);
            }
            let start = authority_boundary(&state.normalized_prefix, state.authority_items)?;
            let canonical_end = start
                .checked_add(canonical_inputs)
                .ok_or(ModelError::BudgetExceeded)?;
            let canonical = state
                .normalized_prefix
                .get(start..canonical_end)
                .ok_or(ModelError::InvalidStream)?;
            let expected = window_with_canonical(items, canonical);
            let end = start
                .checked_add(expected.len())
                .ok_or(ModelError::BudgetExceeded)?;
            if state.wire_prefix.get(start..end) != Some(expected.as_slice()) {
                return Err(ModelError::InvalidStream);
            }
        }
        *self
            .compaction
            .lock()
            .map_err(|_| ModelError::Unavailable)? = Some(InstalledCompaction::Window {
            items: items.to_vec(),
            canonical_inputs,
        });
        Ok(())
    }

    pub(crate) fn expand_compaction(
        &self,
        input: &[Value],
        authority_items: Option<usize>,
    ) -> Result<Vec<Value>, ModelError> {
        let item = self
            .compaction
            .lock()
            .map_err(|_| ModelError::Unavailable)?;
        let Some(installed) = item.as_ref() else {
            return Ok(input.to_vec());
        };
        let authority = authority_boundary(input, authority_items)?;
        if let InstalledCompaction::Window {
            items,
            canonical_inputs,
        } = installed
        {
            let end = authority
                .checked_add(*canonical_inputs)
                .filter(|end| *end <= input.len())
                .ok_or(ModelError::InvalidRequest)?;
            let mut result = input[..authority].to_vec();
            result.extend(window_with_canonical(items, &input[authority..end]));
            result.extend_from_slice(&input[end..]);
            return Ok(result);
        }
        let InstalledCompaction::Opaque {
            item,
            retained_inputs,
        } = installed
        else {
            return Err(ModelError::InvalidStream);
        };
        let position = authority
            .checked_add(*retained_inputs)
            .filter(|position| *position <= input.len())
            .ok_or(ModelError::InvalidRequest)?;
        if input[authority..position]
            .iter()
            .any(|item| item["role"] != "user")
        {
            return Err(ModelError::InvalidRequest);
        }
        let mut result = input[..position].to_vec();
        result.push(item.clone());
        result.extend_from_slice(&input[position..]);
        Ok(result)
    }

    pub(super) fn check_compaction_model(&self, actual: &ModelIdentity) -> Result<(), ModelError> {
        if self
            .compaction
            .lock()
            .map_err(|_| ModelError::Unavailable)?
            .is_some()
            && (actual.provider_id != self.model.provider_id
                || actual.model_id != self.model.model_id)
        {
            return Err(ModelError::UnsupportedCapability);
        }
        Ok(())
    }
}

pub(super) fn authority_prefix(input: &[Value]) -> usize {
    input
        .iter()
        .take_while(|item| item["role"] == "developer" || item["role"] == "system")
        .count()
}

pub(super) fn authority_boundary(
    input: &[Value],
    explicit: Option<usize>,
) -> Result<usize, ModelError> {
    let boundary = explicit.unwrap_or_else(|| authority_prefix(input));
    if boundary > input.len()
        || input[..boundary]
            .iter()
            .any(|item| item["role"] != "developer" && item["role"] != "system")
    {
        return Err(ModelError::InvalidRequest);
    }
    Ok(boundary)
}

fn window_with_canonical(items: &[Value], canonical: &[Value]) -> Vec<Value> {
    let position = crate::agent::model::responses_transport::legacy_canonical_position(items);
    let mut result = items[..position].to_vec();
    result.extend_from_slice(canonical);
    result.extend_from_slice(&items[position..]);
    result
}
