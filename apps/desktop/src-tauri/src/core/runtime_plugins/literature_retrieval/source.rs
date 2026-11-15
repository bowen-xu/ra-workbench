//! Two original static App callbacks. This is private composition, not a plugin API.
use crate::{
    ai::{CancellationSignal, EmbeddingAccess, knowledge_contracts::ConceptRetrievalError},
    storage::AppStore,
};
use std::{future::Future, pin::Pin};
pub(crate) type RetrievalPreviewFuture =
    Pin<Box<dyn Future<Output = Result<ConceptRetrievalPreview, ConceptRetrievalError>> + Send>>;
use research_workbench_native_sdk::concept_retrieval::{
    ConceptRetrievalPreview, ConceptRetrievalPreviewDraft,
};
use rusqlite::Connection;

#[derive(Clone, Copy)]
pub(crate) struct RetrievalPreviewSource {
    pub preview: fn(
        AppStore,
        EmbeddingAccess,
        ConceptRetrievalPreviewDraft,
        CancellationSignal,
    ) -> RetrievalPreviewFuture,
    pub read: fn(&Connection, &str) -> Result<ConceptRetrievalPreview, ConceptRetrievalError>,
}
