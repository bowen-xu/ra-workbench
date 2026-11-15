//! Static analysis composition. Start owns its accepted inputs; repository
//! callbacks borrow the original Core guard. This is an internal App adapter.
use super::analysis_support::AnalysisSupportError;
use crate::ai::paper_analysis_contracts::PaperAnalysisWorkflow;
use rusqlite::Connection;
use std::{future::Future, pin::Pin};
type StartWorkflow = fn(
    crate::storage::AppStore,
    super::analysis_support::AnalysisExecution,
    super::paper_analysis_contracts::PaperAnalysisStartDraft,
    super::CancellationSignal,
) -> Pin<
    Box<dyn Future<Output = Result<PaperAnalysisWorkflow, super::error::AiCommandError>> + Send>,
>;
type ConversationContext =
    fn(&Connection, uuid::Uuid) -> Result<Option<(String, String)>, AnalysisSupportError>;
#[derive(Clone, Copy)]
pub(crate) struct PaperAnalysisSource {
    pub start: StartWorkflow,
    pub workflow: fn(&Connection, &str) -> Result<PaperAnalysisWorkflow, AnalysisSupportError>,
    pub latest:
        fn(&Connection, &str) -> Result<Option<PaperAnalysisWorkflow>, AnalysisSupportError>,
    pub conversation_context: ConversationContext,
    pub interrupt: fn(&Connection) -> Result<(), rusqlite::Error>,
}
