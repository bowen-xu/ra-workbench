//! Preserve original SQL classification and actual source provenance for ordinary chat.
use super::*;
use crate::agent::writing_submission::{WritingSourceDiagnostic, WritingSubmissionError};

#[derive(Debug, Serialize)]
pub(crate) struct ContextSourceFailure {
    code: &'static str,
    reason: SqlImmediateFailure,
    #[serde(rename = "contextSource")]
    context_source: ContextSourceDiagnostic,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextSourceDiagnostic {
    conversation_id: Uuid,
    request_id: Uuid,
    source: WritingSourceDiagnostic,
}
impl Error {
    pub(crate) fn runtime(
        error: WritingSubmissionError,
        conversation: Uuid,
        request: Uuid,
    ) -> Self {
        let (error, source) = error.split_source();
        let storage = match error {
            WritingSubmissionError::Storage(error) => error.into_inner(),
            _ => SqlImmediateFailure::Unavailable(Box::new(SqlImmediateDiagnostic::for_operation(
                request,
                None,
                "context-prepare-source",
                "context_prepare_source_unavailable",
            ))),
        };
        Self::with_source(storage, source, conversation, request)
    }
    pub(crate) fn scope(
        error: crate::data_interface::ContextPrepareScopeFailure,
        conversation: Uuid,
        request: Uuid,
    ) -> Self {
        let (_, source) = WritingSubmissionError::from(error.catalog).split_source();
        Self::with_source(error.error, source, conversation, request)
    }
    fn with_source(
        error: SqlImmediateFailure,
        source: Option<WritingSourceDiagnostic>,
        conversation: Uuid,
        request: Uuid,
    ) -> Self {
        match source {
            Some(source) => Self::Source(Box::new(ContextSourceFailure {
                code: "storage",
                reason: error,
                context_source: ContextSourceDiagnostic {
                    conversation_id: conversation,
                    request_id: request,
                    source,
                },
            })),
            None => Self::Storage(error),
        }
    }
}
