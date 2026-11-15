//! Explicit application bootstrap binding; it resolves no package or provider.
use super::service::LiteratureTraceService;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct NoteTraceBinding {
    pub service: Arc<LiteratureTraceService>,
    pub package_key: String,
}
