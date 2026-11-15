//! Core-internal immutable snapshot port for the original paper SQL consumers.
use crate::ai::{
    paper_reading_contracts::{PaperContextError, PaperContextPreview, PaperContextRequest},
    paper_reading_source::PaperReadingSource,
    structured_resource_validation::Flow,
};
use rusqlite::Connection;

#[derive(Debug)]
pub(crate) enum ReadingNeed {
    Resources,
    Preview(Box<crate::runtime_plugins::literature_context::preview::PreviewCapture>),
    TraceResources,
    Trace(Box<crate::runtime_plugins::literature_trace::data::PreparedTrace>),
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReplayObservation {
    pub receipt: crate::data_interface::SqlImmediateReceipt,
    pub submission: crate::agent::repository::submissions::ObservedSubmission,
    pub scope: Option<(String, String, String, String)>,
    pub grants: bool,
    pub images: Vec<research_agent_core::submission::SubmissionImageReference>,
}
pub(crate) type ReadingResult<T> = Result<T, Flow<PaperContextError, ReadingNeed>>;

/// A missing Native decision exits the original transaction through Flow::Need.
/// The existing SQL owner settles before the managed literature host waits.
pub(crate) trait PaperContextReader: Send {
    fn observe_replay(&mut self, _observation: ReplayObservation) -> ReadingResult<()> {
        Ok(())
    }
    fn preview(
        &mut self,
        db: &Connection,
        request: &PaperContextRequest,
    ) -> ReadingResult<PaperContextPreview>;
    fn retrieval(
        &mut self,
        tx: &rusqlite::Transaction<'_>,
        source: Option<super::retrieval_context::RetrievalTraceSource>,
        selection: &super::retrieval_context::RetrievalSelection,
        paper: &str,
        snapshot: &str,
    ) -> ReadingResult<research_agent_core::submission::ContextText>;
}
impl PaperContextReader for PaperReadingSource {
    fn preview(
        &mut self,
        db: &Connection,
        request: &PaperContextRequest,
    ) -> ReadingResult<PaperContextPreview> {
        (self.preview)(db, request).map_err(Flow::original)
    }
    fn retrieval(
        &mut self,
        tx: &rusqlite::Transaction<'_>,
        source: Option<super::retrieval_context::RetrievalTraceSource>,
        selection: &super::retrieval_context::RetrievalSelection,
        paper: &str,
        snapshot: &str,
    ) -> ReadingResult<research_agent_core::submission::ContextText> {
        let source = source.ok_or_else(|| Flow::original(PaperContextError::Unavailable))?;
        super::retrieval_context::consume_in(
            source,
            tx,
            selection,
            super::retrieval_context::RetrievalScope::Paper {
                paper_id: paper,
                snapshot_id: snapshot,
            },
        )
        .map_err(|_| Flow::original(PaperContextError::Conflict))
    }
}
