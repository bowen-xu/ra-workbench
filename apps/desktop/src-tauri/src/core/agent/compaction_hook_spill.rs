//! Preserve full hook context before publishing a bounded preview. Adapted from
//! Codex hooks/src/output_spill.rs at e363b08c9175ac1cbe5893615dd2cb9ddf95043b,
//! Apache-2.0 / Copyright 2025 OpenAI. Authorized storage remains host-injected.
use research_agent_core::{context::compaction_hooks::*, protocol::ModelIdentity};
use std::{future::Future, pin::Pin, sync::Arc};

#[path = "compaction_hook_spill/file.rs"]
pub mod file;
#[cfg(unix)]
#[path = "compaction_hook_spill/native.rs"]
pub(crate) mod native;
#[cfg(target_os = "macos")]
#[path = "compaction_hook_spill/tool.rs"]
pub(crate) mod tool;
/// Native model/profile snapshot. No renderer serialization or live settings.
#[derive(Clone)]
pub struct HookContextBudget {
    model: ModelIdentity,
    inline_bytes: usize,
}
impl HookContextBudget {
    pub fn new(model: ModelIdentity, inline_bytes: usize) -> Result<Self, HookContextError> {
        if !(256..=1024 * 1024).contains(&inline_bytes)
            || model.provider_id.trim().is_empty()
            || model.model_id.trim().is_empty()
        {
            return Err(HookContextError);
        }
        Ok(Self {
            model,
            inline_bytes,
        })
    }
    pub(crate) fn wrap(
        self,
        model: &ModelIdentity,
        inner: Arc<dyn CompactionHooks>,
        store: Arc<dyn HookSpillStore>,
    ) -> Result<Arc<dyn CompactionHooks>, HookContextError> {
        if &self.model != model {
            return Err(HookContextError);
        }
        Ok(Arc::new(SpillingCompactionHooks::new(
            inner,
            store,
            self.model,
            self.inline_bytes,
        )?))
    }
}
pub type HookSpillFuture<'a> =
    Pin<Box<dyn Future<Output = Result<String, HookContextError>> + Send + 'a>>;
pub type HookSpillReceiptFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), HookContextError>> + Send + 'a>>;
pub type HookArtifactReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<HookArtifactSlice, HookContextError>> + Send + 'a>>;
pub type HookArtifactAuthority = dyn Fn(&research_agent_core::submission::SubmissionRecord) -> Result<(), HookContextError>
    + Send
    + Sync;

/// Native frozen submission and journal cutoff; never deserialize this authority
/// contract from a model or renderer. The artifact UUID is the only locator.
pub struct HookArtifactRead {
    pub submission: Arc<research_agent_core::submission::SubmissionRecord>,
    pub through: u64,
    pub artifact_id: uuid::Uuid,
    pub offset: usize,
    pub limit: usize,
}
#[derive(Debug, PartialEq, Eq)]
pub struct HookArtifactSlice {
    pub text: String,
    pub total_bytes: usize,
    pub next_offset: Option<usize>,
    pub sha256: String,
}
pub trait HookArtifactReader: Send + Sync {
    /// Require current native host permission in addition to the persisted grant.
    /// Read limits are 4..=65536 bytes, with exact UTF-8 boundary offsets.
    fn read(
        &self,
        request: HookArtifactRead,
        authority: Arc<HookArtifactAuthority>,
    ) -> HookArtifactReadFuture<'_>;
}

/// Returns a recovery locator only after full output is durably saved. The host
/// must authorize both storage and subsequent tool access; no renderer paths.
pub trait HookSpillStore: Send + Sync {
    fn save(&self, request: CompactionHookRequest, text: String) -> HookSpillFuture<'_>;

    /// Bind the exact published preview to the durable output before returning
    /// context to the task owner. Native stores can persist a pending receipt;
    /// only the owner's later context transaction may activate a read grant.
    /// A locator occurring in hook-authored text is never proof of permission.
    /// Already authorized filesystem adapters need no additional receipt.
    fn record_preview(
        &self,
        _request: CompactionHookRequest,
        _locator: String,
        _preview: String,
    ) -> HookSpillReceiptFuture<'_> {
        Box::pin(async { Ok(()) })
    }
}

/// Byte budget is an injected model/profile decision, not an exact tokenizer.
/// Does not modify command outcomes or implement a second execution loop.
pub struct SpillingCompactionHooks {
    inner: Arc<dyn CompactionHooks>,
    store: Arc<dyn HookSpillStore>,
    model: ModelIdentity,
    inline_bytes: usize,
}
impl SpillingCompactionHooks {
    pub fn new(
        inner: Arc<dyn CompactionHooks>,
        store: Arc<dyn HookSpillStore>,
        model: ModelIdentity,
        inline_bytes: usize,
    ) -> Result<Self, HookContextError> {
        if !(256..=1024 * 1024).contains(&inline_bytes)
            || model.provider_id.trim().is_empty()
            || model.model_id.trim().is_empty()
        {
            return Err(HookContextError);
        }
        Ok(Self {
            inner,
            store,
            model,
            inline_bytes,
        })
    }

    async fn prepare(
        &self,
        request: CompactionHookRequest,
        result: CompactionHookResult,
    ) -> Result<CompactionHookResult, HookContextError> {
        if request.model != self.model {
            return Err(HookContextError);
        }
        let mut result = self.inner.prepare_context(request.clone(), result).await?;
        if result.additional_contexts.is_empty() {
            return Ok(result);
        }
        if request.stage != CompactionHookStage::SessionStartCompact
            || !matches!(
                result.outcome,
                CompactionHookOutcome::Continue | CompactionHookOutcome::Stop
            )
            || result.additional_contexts.len() > 256
            || result
                .additional_contexts
                .iter()
                .try_fold(0usize, |bytes, text| bytes.checked_add(text.len()))
                .is_none_or(|bytes| bytes > 32 * 1024 * 1024)
        {
            return Err(HookContextError);
        }
        let budget = self
            .inline_bytes
            .min(1024 * 1024 / result.additional_contexts.len());
        let mut prepared = Vec::with_capacity(result.additional_contexts.len());
        for text in result.additional_contexts {
            if text.len() <= budget {
                prepared.push(text);
                continue;
            }
            let locator = self.store.save(request.clone(), text.clone()).await?;
            let context = preview(&text, &locator, budget)?;
            self.store
                .record_preview(request.clone(), locator, context.clone())
                .await?;
            prepared.push(context);
        }
        result.additional_contexts = prepared;
        Ok(result)
    }
}
impl CompactionHooks for SpillingCompactionHooks {
    fn notify(&self, request: &CompactionHookRequest, event: CompactionHookNotification) {
        self.inner.notify(request, event);
    }
    fn run(&self, request: CompactionHookRequest) -> CompactionHookFuture<'_> {
        self.inner.run(request)
    }
    fn run_with_context(&self, request: CompactionHookRequest) -> CompactionHookResultFuture<'_> {
        self.inner.run_with_context(request)
    }
    fn handler_ids(&self, stage: CompactionHookStage) -> Vec<uuid::Uuid> {
        self.inner.handler_ids(stage)
    }
    fn handler_ids_for(&self, request: &CompactionHookRequest) -> Vec<uuid::Uuid> {
        self.inner.handler_ids_for(request)
    }
    fn validate_request(&self, request: &CompactionHookRequest) -> Result<(), HookContextError> {
        self.inner.validate_request(request)
    }
    fn run_with_handler_observer(
        &self,
        request: CompactionHookRequest,
        observer: Arc<dyn HookHandlerObserver>,
    ) -> CompactionHookResultFuture<'_> {
        self.inner.run_with_handler_observer(request, observer)
    }
    fn prepare_context(
        &self,
        request: CompactionHookRequest,
        result: CompactionHookResult,
    ) -> HookContextPreparation<'_> {
        Box::pin(self.prepare(request, result))
    }
}

fn preview(text: &str, locator: &str, budget: usize) -> Result<String, HookContextError> {
    // Keep an unambiguous one-line locator; never truncate a recovery path.
    if locator.is_empty() || locator.contains(['\n', '\r', '\0']) {
        return Err(HookContextError);
    }
    let marker = "\n… hook output truncated …\n";
    let footer = format!("\n\nFull hook output saved to: {locator}");
    let remaining = budget
        .checked_sub(marker.len() + footer.len())
        .ok_or(HookContextError)?;
    let mut head = remaining / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len().saturating_sub(remaining - head);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    Ok(format!(
        "{}{marker}{}{footer}",
        &text[..head],
        &text[tail..]
    ))
}

#[cfg(test)]
#[path = "compaction_hook_spill/tests.rs"]
mod tests;
