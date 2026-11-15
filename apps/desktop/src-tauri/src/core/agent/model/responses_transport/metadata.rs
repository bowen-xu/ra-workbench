//! Provider-specific metadata is frozen with input preparation, not recomputed
//! during credential refresh or HTTP retry. Standard Platform requests retain
//! their own API contract rather than receive Codex client_metadata fields.
use super::*;
use crate::agent::model::responses_metadata::ResponsesRequestMetadata;

impl ResponsesTransport {
    pub(super) async fn prepare_body_input(
        &self,
        body: &mut Value,
        request: &ModelRequest,
        account: Uuid,
        input: Vec<Value>,
        captured: super::super::responses_metadata::CapturedResponsesInput,
    ) -> Result<Option<ResponsesRequestMetadata>, ModelError> {
        let prepared = self
            .receipts
            .prepare_canonical_input(account, request.model.clone(), input, captured)
            .await?;
        body["input"] = Value::Array(prepared.input);
        if matches!(self.kind, ResponsesEndpointKind::ChatGpt) {
            if let Some(metadata) = &prepared.metadata {
                metadata.apply_body(body);
            }
            Ok(prepared.metadata)
        } else {
            Ok(None)
        }
    }
}
