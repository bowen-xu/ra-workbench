//! Response identity is observed independently from optional model metadata.
//! Header model extraction follows fixed Codex codex-api/src/sse/responses.rs.
//! Copyright 2025 OpenAI. Adapted under Apache-2.0.
use super::*;

impl ResponsesDecoder {
    pub(super) fn observe_identity(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelStreamEvent>, ModelError> {
        let response = &event["response"];
        for id in [
            optional_string(response, "id")?,
            optional_string(event, "response_id")?,
        ]
        .into_iter()
        .flatten()
        {
            if self.response_id.as_deref().is_some_and(|known| known != id) {
                return Err(ModelError::InvalidStream);
            }
            self.response_id = Some(id.to_owned());
        }
        // Fixed Codex explicitly ignores response.model: it may be a requested
        // alias rather than the effective server model. SSE response headers
        // take precedence over top-level websocket metadata headers.
        let observed_model =
            header_model(&response["headers"]).or_else(|| header_model(&event["headers"]));
        let new_model = observed_model
            .as_ref()
            .is_some_and(|model| self.actual_model.as_ref() != Some(model));
        if let Some(model) = observed_model {
            self.actual_model = Some(model);
        }
        if !self.started {
            let Some(response_id) = self.response_id.clone() else {
                return Ok(vec![]);
            };
            self.started = true;
            return Ok(vec![match &self.actual_model {
                Some(model) => ModelStreamEvent::Started {
                    response_id,
                    actual_model: self.model_identity(model),
                },
                None => ModelStreamEvent::StartedWithoutModel { response_id },
            }]);
        }
        if new_model {
            return Ok(vec![ModelStreamEvent::ModelIdentityObserved(
                self.model_identity(
                    self.actual_model
                        .as_deref()
                        .ok_or(ModelError::InvalidStream)?,
                ),
            )]);
        }
        Ok(vec![])
    }

    fn model_identity(&self, model: &str) -> ModelIdentity {
        ModelIdentity {
            provider_id: self.provider.clone(),
            model_id: model.to_owned(),
            revision: None,
        }
    }
}

fn header_model(value: &Value) -> Option<String> {
    value.as_object()?.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("openai-model") || name.eq_ignore_ascii_case("x-openai-model")
        {
            header_string(value)
                .filter(|model| !model.is_empty())
                .map(str::to_owned)
        } else {
            None
        }
    })
}

fn header_string(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value),
        Value::Array(values) => values.first().and_then(header_string),
        _ => None,
    }
}

fn optional_string<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, ModelError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        Some(_) => Err(ModelError::InvalidStream),
    }
}
