//! Native immutable output storage; reuses the app's content lock and staging
//! recovery journal. Neither locators nor preview text grant filesystem access.
use super::*;
use crate::{
    agent::{
        repository::hook_spills,
        workspace::{ProposalObjects, file_version, object_staging},
    },
    storage::AppStore,
};
use rusqlite::TransactionBehavior;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct NativeHookSpillStore {
    store: AppStore,
}
impl NativeHookSpillStore {
    pub(crate) fn new(store: AppStore) -> Self {
        Self { store }
    }
    fn save_full(
        &self,
        request: &CompactionHookRequest,
        text: &str,
    ) -> Result<String, HookContextError> {
        if text.len() > 32 * 1024 * 1024 {
            return Err(HookContextError);
        }
        let _content = self.store.content_write().map_err(|_| HookContextError)?;
        {
            let db = self.store.connection().map_err(|_| HookContextError)?;
            hook_spills::validate_request(&db, request).map_err(|_| HookContextError)?;
        }
        self.store
            .ensure_content_import_capacity(text.len() as u64)
            .map_err(|_| HookContextError)?;
        let root = self
            .store
            .content_directory()
            .canonicalize()
            .map_err(|_| HookContextError)?;
        let objects = ProposalObjects::open(&root).map_err(|_| HookContextError)?;
        let version = file_version(text.as_bytes(), false);
        object_staging::stage(&self.store, &objects, &version, text.as_bytes())
            .map_err(|_| HookContextError)?;
        let id = Uuid::new_v4();
        let mut db = self.store.connection().map_err(|_| HookContextError)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| HookContextError)?;
        hook_spills::save_receipt(&tx, request, id, &version).map_err(|_| HookContextError)?;
        tx.commit().map_err(|_| HookContextError)?;
        Ok(hook_spills::locator(id))
    }
}
impl HookSpillStore for NativeHookSpillStore {
    fn save(&self, request: CompactionHookRequest, text: String) -> HookSpillFuture<'_> {
        let store = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || store.save_full(&request, &text))
                .await
                .map_err(|_| HookContextError)?
        })
    }
    fn record_preview(
        &self,
        request: CompactionHookRequest,
        locator: String,
        preview: String,
    ) -> HookSpillReceiptFuture<'_> {
        let store = self.store.clone();
        Box::pin(async move {
            let id = parse_locator(&locator)?;
            tokio::task::spawn_blocking(move || {
                let mut db = store.connection().map_err(|_| HookContextError)?;
                let tx = db
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|_| HookContextError)?;
                hook_spills::bind_preview(&tx, &request, id, &preview)
                    .map_err(|_| HookContextError)?;
                tx.commit().map_err(|_| HookContextError)
            })
            .await
            .map_err(|_| HookContextError)?
        })
    }
}
fn parse_locator(locator: &str) -> Result<Uuid, HookContextError> {
    let id = locator
        .strip_prefix("hook-artifact://")
        .ok_or(HookContextError)?;
    let id = Uuid::parse_str(id).map_err(|_| HookContextError)?;
    if hook_spills::locator(id) != locator || id.is_nil() {
        return Err(HookContextError);
    }
    Ok(id)
}

impl HookArtifactReader for NativeHookSpillStore {
    fn read(
        &self,
        request: HookArtifactRead,
        authority: Arc<HookArtifactAuthority>,
    ) -> HookArtifactReadFuture<'_> {
        let store = self.store.clone();
        Box::pin(async move {
            if !(4..=65536).contains(&request.limit) || request.artifact_id.is_nil() {
                return Err(HookContextError);
            }
            tokio::task::spawn_blocking(move || {
                authority(&request.submission)?;
                let _content = store.content_read().map_err(|_| HookContextError)?;
                let resolve = || {
                    let db = store.connection().map_err(|_| HookContextError)?;
                    hook_spills::resolve(
                        &db,
                        request.submission.conversation_id,
                        request.artifact_id,
                        &request.submission.input.model,
                        request.through,
                    )
                    .map_err(|_| HookContextError)
                };
                let (version, path) = resolve()?;
                if version.byte_size > 32 * 1024 * 1024 {
                    return Err(HookContextError);
                }
                let root = store
                    .content_directory()
                    .canonicalize()
                    .map_err(|_| HookContextError)?;
                let objects = ProposalObjects::open(&root).map_err(|_| HookContextError)?;
                let bytes = objects
                    .read_registered(&version, &path)
                    .map_err(|_| HookContextError)?;
                let text = String::from_utf8(bytes).map_err(|_| HookContextError)?;
                if !text.is_char_boundary(request.offset) {
                    return Err(HookContextError);
                }
                let mut end = request.offset.saturating_add(request.limit).min(text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                let response = HookArtifactSlice {
                    text: text[request.offset..end].into(),
                    total_bytes: text.len(),
                    next_offset: (end < text.len()).then_some(end),
                    sha256: version.sha256.clone(),
                };
                // No DB mutex is held during file I/O or host authority callbacks.
                // A rollback, archive, deletion or revocation during the read must
                // not return the already-buffered research text.
                if resolve()? != (version, path) {
                    return Err(HookContextError);
                }
                authority(&request.submission)?;
                Ok(response)
            })
            .await
            .map_err(|_| HookContextError)?
        })
    }
}

#[cfg(test)]
#[path = "native/tests.rs"]
pub(super) mod tests;
