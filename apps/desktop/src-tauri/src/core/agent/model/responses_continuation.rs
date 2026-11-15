//! Bounded native Responses replay for one owner/model scope. No opaque provider
//! items are inserted into user-visible normalized history.
use super::responses_transport::{ResponsesReceipt, ResponsesReceiptSink};
use research_agent_core::{
    model::{ModelError, ModelFuture},
    protocol::ModelIdentity,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Mutex;
use uuid::Uuid;

/// In-memory turn-local owner. Durable recovery must restore a validated
/// checkpoint rather than construct a fresh owner over an ongoing Responses run.
/// Caller creates a new instance for each turn/account/context scope.
pub struct ResponsesContinuation {
    scope_id: Uuid,
    model: ModelIdentity,
    max_bytes: usize,
    state: Mutex<Option<Checkpoint>>,
    credential_scope: Mutex<Option<Uuid>>,
    // Rebound from the installed window by its SQLite owner on each transaction.
    // Never serialized into normalized/UI history or treated as a response.
    compaction: Mutex<Option<InstalledCompaction>>,
}
enum InstalledCompaction {
    Opaque {
        item: Value,
        retained_inputs: usize,
    },
    Window {
        items: Vec<Value>,
        canonical_inputs: usize,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    request_id: Uuid,
    #[serde(default)]
    origin_scope_id: Option<Uuid>,
    actual_model: ModelIdentity,
    normalized_prefix: Vec<Value>,
    wire_prefix: Vec<Value>,
    #[serde(default)]
    authority_items: Option<usize>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCheckpoint {
    schema_version: u32,
    scope_id: Uuid,
    model: ModelIdentity,
    credential_scope: Option<Uuid>,
    checkpoint: Option<Checkpoint>,
}
impl ResponsesContinuation {
    pub fn new(scope_id: Uuid, model: ModelIdentity, max_bytes: usize) -> Result<Self, ModelError> {
        if scope_id.is_nil()
            || max_bytes == 0
            || model.provider_id.trim().is_empty()
            || model.model_id.trim().is_empty()
        {
            return Err(ModelError::InvalidRequest);
        }
        Ok(Self {
            scope_id,
            model,
            max_bytes,
            state: Mutex::new(None),
            credential_scope: Mutex::new(None),
            compaction: Mutex::new(None),
        })
    }
    pub fn scope_id(&self) -> Uuid {
        self.scope_id
    }
    #[cfg(test)]
    pub(crate) fn saved_request_id(&self) -> Result<Option<Uuid>, ModelError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ModelError::Unavailable)?
            .as_ref()
            .filter(|checkpoint| checkpoint.origin_scope_id.is_none())
            .map(|checkpoint| checkpoint.request_id))
    }
    /// Native storage only: contains provider continuation data, never UI JSON.
    pub(crate) fn encode_checkpoint(&self) -> Result<String, ModelError> {
        let credentials = self
            .credential_scope
            .lock()
            .map_err(|_| ModelError::Unavailable)?;
        let state = self.state.lock().map_err(|_| ModelError::Unavailable)?;
        let schema = if state
            .as_ref()
            .is_some_and(|checkpoint| checkpoint.authority_items.is_some())
        {
            3
        } else {
            2
        };
        let bytes =
            serde_json::to_string(&json!({"schema_version":schema,"scope_id":self.scope_id,
            "model":self.model,"credential_scope":*credentials,"checkpoint":*state}))
            .map_err(|_| ModelError::InvalidRequest)?;
        if bytes.len() > self.max_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        Ok(bytes)
    }
    pub(crate) fn restore(
        scope_id: Uuid,
        model: ModelIdentity,
        max_bytes: usize,
        encoded: &str,
    ) -> Result<Self, ModelError> {
        if encoded.len() > max_bytes {
            return Err(ModelError::BudgetExceeded);
        }
        let stored: StoredCheckpoint =
            serde_json::from_str(encoded).map_err(|_| ModelError::InvalidStream)?;
        if !matches!(stored.schema_version, 1..=3)
            || stored.scope_id != scope_id
            || stored.model != model
            || stored.credential_scope.is_some_and(|id| id.is_nil())
        {
            return Err(ModelError::InvalidStream);
        }
        if let Some(checkpoint) = &stored.checkpoint
            && (checkpoint.request_id.is_nil()
                || stored.credential_scope.is_none()
                || checkpoint.actual_model.provider_id != model.provider_id
                || checkpoint.actual_model.model_id.trim().is_empty()
                || checkpoint.normalized_prefix.is_empty()
                || checkpoint.wire_prefix.is_empty())
        {
            return Err(ModelError::InvalidStream);
        }
        if stored.checkpoint.as_ref().is_some_and(|checkpoint| {
            checkpoint
                .origin_scope_id
                .is_some_and(|id| id.is_nil() || id == scope_id)
                || (stored.schema_version == 1 && checkpoint.origin_scope_id.is_some())
        }) {
            return Err(ModelError::InvalidStream);
        }
        let result = Self::new(scope_id, model, max_bytes)?;
        if let Some(checkpoint) = &stored.checkpoint {
            if stored.schema_version == 3 && checkpoint.authority_items.is_none() {
                return Err(ModelError::InvalidStream);
            }
            compaction::authority_boundary(
                &checkpoint.normalized_prefix,
                checkpoint.authority_items,
            )
            .map_err(|_| ModelError::InvalidStream)?;
        }
        *result
            .credential_scope
            .lock()
            .map_err(|_| ModelError::Unavailable)? = stored.credential_scope;
        *result.state.lock().map_err(|_| ModelError::Unavailable)? = stored.checkpoint;
        Ok(result)
    }
    fn check_credentials(&self, identity: Uuid, bind: bool) -> Result<(), ModelError> {
        if identity.is_nil() {
            return Err(ModelError::Authentication);
        }
        let mut current = self
            .credential_scope
            .lock()
            .map_err(|_| ModelError::Unavailable)?;
        match *current {
            Some(existing) if existing == identity => Ok(()),
            None if bind => {
                *current = Some(identity);
                Ok(())
            }
            _ => Err(ModelError::Authentication),
        }
    }
    fn expand(
        &self,
        state: Option<&Checkpoint>,
        model: &ModelIdentity,
        input: &[Value],
        authority_items: Option<usize>,
    ) -> Result<Vec<Value>, ModelError> {
        if model != &self.model {
            return Err(ModelError::InvalidRequest);
        }
        let result = match state {
            None => self.expand_compaction(input, authority_items)?,
            Some(state) if state.origin_scope_id.is_some() => {
                // A new turn restores current host authority. Replace exactly
                // the known normalized authority slots, never scan/prune provider
                // window items that may themselves have developer/system roles.
                let old = compaction::authority_boundary(
                    &state.normalized_prefix,
                    state.authority_items,
                )?;
                let current = compaction::authority_boundary(input, authority_items)?;
                if state.wire_prefix.get(..old) != Some(&state.normalized_prefix[..old]) {
                    return Err(ModelError::InvalidStream);
                }
                let history = &state.normalized_prefix[old..];
                if !input[current..].starts_with(history) {
                    return Err(ModelError::InvalidRequest);
                }
                let mut result = input[..current].to_vec();
                result.extend_from_slice(&state.wire_prefix[old..]);
                result.extend_from_slice(&input[current + history.len()..]);
                result
            }
            Some(state) => {
                if !input.starts_with(&state.normalized_prefix) {
                    return Err(ModelError::InvalidRequest);
                }
                let mut result = state.wire_prefix.clone();
                result.extend_from_slice(&input[state.normalized_prefix.len()..]);
                result
            }
        };
        self.budget(&result)?;
        Ok(result)
    }
    fn budget(&self, value: &impl serde::Serialize) -> Result<(), ModelError> {
        let bytes = serde_json::to_vec(value).map_err(|_| ModelError::InvalidRequest)?;
        if bytes.len() > self.max_bytes {
            Err(ModelError::BudgetExceeded)
        } else {
            Ok(())
        }
    }
}

#[path = "responses_continuation/compaction.rs"]
mod compaction;
impl ResponsesReceiptSink for ResponsesContinuation {
    fn prepare_with_authority(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        kind: super::responses_metadata::ResponsesInputKind,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move {
            self.prepare_scoped_sync(
                account,
                model,
                input,
                kind.authority_items(),
                kind.text_compaction(),
            )
        })
    }
    fn prepare_with_metadata(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        kind: super::responses_metadata::ResponsesInputKind,
    ) -> ModelFuture<'_, super::responses_metadata::PreparedResponsesInput> {
        Box::pin(async move {
            Ok(super::responses_metadata::PreparedResponsesInput {
                input: self.prepare_scoped_sync(
                    account,
                    model,
                    input,
                    kind.authority_items(),
                    kind.text_compaction(),
                )?,
                metadata: None,
            })
        })
    }
    fn prepare_text_compaction_input(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move { self.prepare_text_sync(account, model, input) })
    }
    fn prepare_input(
        &self,
        credential_scope_id: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move { self.prepare_sync(credential_scope_id, model, input) })
    }
    fn preserve(&self, receipt: ResponsesReceipt) -> ModelFuture<'_, ()> {
        Box::pin(async move { self.preserve_sync(receipt) })
    }
}
impl ResponsesContinuation {
    pub(crate) fn prepare_sync(
        &self,
        credential_scope_id: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
    ) -> Result<Vec<Value>, ModelError> {
        self.prepare_scoped_sync(credential_scope_id, model, input, None, false)
    }
    /// Summary requests may deliberately remove oldest history. Never relax the
    /// prefix contract of ordinary continuation or persist this candidate here.
    pub(crate) fn prepare_text_sync(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
    ) -> Result<Vec<Value>, ModelError> {
        self.prepare_scoped_sync(account, model, input, None, true)
    }
    pub(crate) fn prepare_scoped_sync(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        authority_items: Option<usize>,
        text_compaction: bool,
    ) -> Result<Vec<Value>, ModelError> {
        compaction::authority_boundary(&input, authority_items)?;
        self.check_credentials(account, true)?;
        let state = self.state.lock().map_err(|_| ModelError::Unavailable)?;
        match self.expand(state.as_ref(), &model, &input, authority_items) {
            Err(ModelError::InvalidRequest) if text_compaction => {
                self.expand(None, &model, &input, authority_items)
            }
            result => result,
        }
    }
    pub(crate) fn preserve_sync(&self, receipt: ResponsesReceipt) -> Result<(), ModelError> {
        compaction::authority_boundary(&receipt.normalized_input, receipt.authority_items)?;
        if receipt.request_id.is_nil() || receipt.model.model_id.trim().is_empty() {
            return Err(ModelError::InvalidStream);
        }
        self.check_credentials(receipt.credential_scope_id, false)?;
        self.check_compaction_model(&receipt.model)?;
        let mut state = self.state.lock().map_err(|_| ModelError::Unavailable)?;
        if let Some(previous) = state.as_ref().filter(|state| {
            state.origin_scope_id.is_none() && state.request_id == receipt.request_id
        }) {
            let mut normalized = receipt.normalized_input.clone();
            normalized.extend(project_output(&receipt.output)?);
            let mut wire = receipt.input.clone();
            wire.extend(receipt.output);
            return if receipt.requested_model == self.model
                && previous.actual_model == receipt.model
                && previous.normalized_prefix == normalized
                && previous.wire_prefix == wire
                && previous.authority_items == receipt.authority_items
            {
                Ok(())
            } else {
                Err(ModelError::InvalidRequest)
            };
        }
        if self.expand(
            state.as_ref(),
            &receipt.requested_model,
            &receipt.normalized_input,
            receipt.authority_items,
        )? != receipt.input
        {
            return Err(ModelError::InvalidRequest);
        }
        if receipt.model.provider_id != self.model.provider_id {
            return Err(ModelError::InvalidStream);
        }
        let mut normalized_prefix = receipt.normalized_input;
        normalized_prefix.extend(project_output(&receipt.output)?);
        let mut wire_prefix = receipt.input;
        wire_prefix.extend(receipt.output);
        // Bound the combined retained representation, not just each half.
        self.budget(&(&normalized_prefix, &wire_prefix))?;
        *state = Some(Checkpoint {
            request_id: receipt.request_id,
            origin_scope_id: None,
            actual_model: receipt.model,
            normalized_prefix,
            wire_prefix,
            authority_items: receipt.authority_items,
        });
        Ok(())
    }
}
/// Project the same ordered assistant messages/calls used by the shared runner.
/// Native items remain intact in wire_prefix; normalized history retains each
/// message boundary and known phase so continuation can prove its exact prefix.
fn project_output(output: &[Value]) -> Result<Vec<Value>, ModelError> {
    let mut result = Vec::new();
    for item in output {
        match item["type"].as_str() {
            Some("reasoning") => {}
            Some("message") => {
                if item["role"] != "assistant" {
                    return Err(ModelError::InvalidStream);
                }
                let mut text = String::new();
                for part in item["content"]
                    .as_array()
                    .ok_or(ModelError::InvalidStream)?
                {
                    let key = match part["type"].as_str() {
                        Some("output_text") => "text",
                        Some("refusal") => "refusal",
                        _ => return Err(ModelError::UnsupportedResponseType),
                    };
                    text.push_str(part[key].as_str().ok_or(ModelError::InvalidStream)?);
                }
                if !text.is_empty() {
                    let mut message = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]});
                    match item.get("phase") {
                        None | Some(Value::Null) => {}
                        Some(Value::String(phase))
                            if matches!(phase.as_str(), "commentary" | "final_answer") =>
                        {
                            message["phase"] = json!(phase);
                        }
                        Some(Value::String(_)) => {}
                        Some(_) => return Err(ModelError::InvalidStream),
                    }
                    result.push(message);
                }
            }
            Some("function_call") => {
                let args: Value = serde_json::from_str(
                    item["arguments"]
                        .as_str()
                        .ok_or(ModelError::InvalidStream)?,
                )
                .map_err(|_| ModelError::InvalidStream)?;
                if !args.is_object()
                    || item["call_id"].as_str().is_none_or(str::is_empty)
                    || item["name"].as_str().is_none_or(str::is_empty)
                {
                    return Err(ModelError::InvalidStream);
                }
                result.push(json!({"type":"function_call","call_id":item["call_id"],"name":item["name"],"arguments":args.to_string()}));
            }
            _ => return Err(ModelError::UnsupportedResponseType),
        }
    }
    Ok(result)
}

#[path = "responses_continuation/captured.rs"]
mod captured;
pub(crate) use captured::CapturedLegacyInput;
#[path = "responses_continuation/inheritance.rs"]
mod inheritance;
#[cfg(test)]
#[path = "responses_continuation/tests.rs"]
mod tests;
