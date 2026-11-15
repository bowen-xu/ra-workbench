//! Native preparation ports borrow the caller's SQL snapshot.
//! They do not expose a database connection to an independently built plugin.
use research_agent_core::submission::ContextText;
pub use research_workbench_native_sdk::writing_context::{
    WritingContextBuffer, WritingContextError, WritingContextFile, WritingContextRequest,
    WritingContextSelection,
};
use uuid::Uuid;

pub(crate) type SelectedWritingContext = fn(
    &rusqlite::Connection,
    &std::path::Path,
    WritingContextRequest,
) -> Result<Vec<ContextText>, WritingContextError>;
pub(crate) type CurrentWritingFile = fn(
    &rusqlite::Connection,
    &std::path::Path,
    Uuid,
    &str,
) -> Result<Vec<ContextText>, WritingContextError>;

#[derive(Clone, Copy)]
pub(crate) struct WritingContextSource {
    pub selected: SelectedWritingContext,
    pub current_file: CurrentWritingFile,
}

pub(crate) type ContextNeed = Box<research_workbench_native_sdk::writing_project::Input>;

pub(crate) type ContextRead<T> =
    Result<T, crate::ai::structured_resource_validation::Flow<WritingContextError, ContextNeed>>;

/// Request-local read policy. A missing Native decision leaves the original
/// SQL transaction through Flow::Need; the owner settles it before any await.
pub(crate) trait WritingContextReader: Send {
    fn resource_scope(&self) -> Option<super::writing_resources::WritingScope<'_>> {
        None
    }
    fn selected(
        &mut self,
        db: &rusqlite::Connection,
        root: &std::path::Path,
        request: WritingContextRequest,
    ) -> ContextRead<Vec<ContextText>>;
    fn current_file(
        &mut self,
        db: &rusqlite::Connection,
        root: &std::path::Path,
        manuscript: Uuid,
        path: &str,
    ) -> ContextRead<Vec<ContextText>>;
}
