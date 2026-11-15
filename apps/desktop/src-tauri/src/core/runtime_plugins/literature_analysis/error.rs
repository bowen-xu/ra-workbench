#[derive(Debug, thiserror::Error)]
pub(crate) enum AnalysisError {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("{0}")]
    InvalidInput(String),
    #[error("analysis temporarily unavailable")]
    TemporaryUnavailable,
    #[error(transparent)]
    Core(#[from] crate::ai::analysis_support::AnalysisSupportError),
}

impl From<research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError>
    for AnalysisError
{
    fn from(
        error: research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError,
    ) -> Self {
        use research_workbench_native_sdk::analysis_workflow::AnalysisWorkflowError;
        match error {
            AnalysisWorkflowError::InvalidInput(message) => Self::InvalidInput(message),
            AnalysisWorkflowError::TemporaryUnavailable => Self::TemporaryUnavailable,
        }
    }
}
