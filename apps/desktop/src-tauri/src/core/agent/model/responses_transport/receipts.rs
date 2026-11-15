//! Canonical input preparation and read-only legacy continuation port.
use super::*;

/// Completed output is admitted through the runner's common ItemAccepted event.
/// This port expands installed windows and legacy checkpoints before HTTP; new
/// transports never preserve a second cumulative response receipt.
pub trait ResponsesReceiptSink: Send + Sync {
    /// New canonical history is already complete. Native storage may consult
    /// legacy material only through these frozen identities; it must not save
    /// a new accumulated copy of the input or response.
    fn prepare_canonical_input(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        captured: super::super::responses_metadata::CapturedResponsesInput,
    ) -> ModelFuture<'_, PreparedResponsesInput> {
        self.prepare_with_metadata(
            account,
            model,
            input,
            ResponsesInputKind::Scoped {
                authority_items: captured.authority_items,
                text_compaction: captured.text_compaction,
            },
        )
    }
    /// Standard Platform preparation preserves authority without Codex metadata.
    fn prepare_with_authority(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        kind: ResponsesInputKind,
    ) -> ModelFuture<'_, Vec<Value>> {
        Box::pin(async move {
            if kind.text_compaction() {
                self.prepare_text_compaction_input(account, model, input)
                    .await
            } else {
                self.prepare_input(account, model, input).await
            }
        })
    }
    /// Codex backend preparation. Native storage overrides this to freeze input
    /// and metadata in one transaction; other injected sinks omit metadata.
    fn prepare_with_metadata(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
        kind: ResponsesInputKind,
    ) -> ModelFuture<'_, PreparedResponsesInput> {
        Box::pin(async move {
            let input = if kind.text_compaction() {
                self.prepare_text_compaction_input(account, model, input)
                    .await?
            } else {
                self.prepare_input(account, model, input).await?
            };
            Ok(PreparedResponsesInput {
                input,
                metadata: None,
            })
        })
    }
    fn prepare_input(
        &self,
        credential_scope_id: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>>;
    fn preserve(&self, receipt: ResponsesReceipt) -> ModelFuture<'_, ()>;
    fn prepare_text_compaction_input(
        &self,
        account: Uuid,
        model: ModelIdentity,
        input: Vec<Value>,
    ) -> ModelFuture<'_, Vec<Value>> {
        self.prepare_input(account, model, input)
    }
}
