//! Shared admission of an already-previewed concept-retrieval trace.
//!
//! Retrieval itself remains owned by `knowledge::concept_retrieval`. This
//! adapter only binds a consumed trace to a host scope and turns it into the
//! common frozen submission source used by writing and reading.
use research_agent_core::submission::ContextText;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetrievalSelection {
    pub trace_id: String,
    pub query: String,
}

pub enum RetrievalScope<'a> {
    Conversation {
        conversation_id: &'a str,
    },
    Paper {
        paper_id: &'a str,
        snapshot_id: &'a str,
    },
    Manuscript {
        manuscript_id: &'a str,
    },
    Note {
        resource_id: &'a str,
        anchor_key: &'a str,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RetrievalContextError {
    #[error("retrieval selection is invalid")]
    Invalid,
    #[error("retrieval trace is unavailable or no longer matches its scope")]
    Conflict,
}

/// Owned result of the existing preview trace loader. This is not a new trace
/// model or persistent representation; App moves the original two values.
pub(crate) struct LoadedRetrievalContext {
    pub trace_id: String,
    pub context_text: String,
}
type LoadTrace = fn(
    &Connection,
    &str,
    Option<&str>,
    &str,
) -> Result<
    LoadedRetrievalContext,
    crate::agent::repository::SqlRepositoryError<RetrievalContextError>,
>;
type MarkTrace =
    fn(
        &Connection,
        &str,
    ) -> Result<(), crate::agent::repository::SqlRepositoryError<RetrievalContextError>>;
/// Required static domain operations, borrowing the caller's original SQL
/// transaction/connection. No runtime, connection getter or commit escapes.
#[derive(Clone, Copy)]
pub(crate) struct RetrievalTraceSource {
    pub load_for_scope: LoadTrace,
    pub mark_sent: MarkTrace,
}

impl RetrievalSelection {
    pub fn validate(&self) -> Result<(), RetrievalContextError> {
        if self.trace_id.trim().is_empty()
            || self.trace_id.len() > 160
            || self.trace_id.chars().any(char::is_control)
            || self.query.trim().is_empty()
            || self.query.len() > 16_384
            || self.query.chars().any(char::is_control)
        {
            return Err(RetrievalContextError::Invalid);
        }
        Ok(())
    }
}

pub fn consume_in(
    trace_source: RetrievalTraceSource,
    connection: &Connection,
    selection: &RetrievalSelection,
    scope: RetrievalScope<'_>,
) -> Result<ContextText, RetrievalContextError> {
    consume_in_sql(trace_source, connection, selection, scope)
        .map_err(|cause| cause.domain_or(RetrievalContextError::Conflict))
}

pub(crate) fn consume_in_sql(
    trace_source: RetrievalTraceSource,
    connection: &Connection,
    selection: &RetrievalSelection,
    scope: RetrievalScope<'_>,
) -> Result<ContextText, crate::agent::repository::SqlRepositoryError<RetrievalContextError>> {
    use crate::agent::repository::SqlRepositoryError as E;
    selection.validate().map_err(E::Domain)?;
    let (scope_id, source_resource_id, anchor) = match scope {
        RetrievalScope::Conversation { conversation_id } => (
            None,
            "global-research",
            serde_json::json!({
                "conversationId":conversation_id,"traceId":selection.trace_id,"query":selection.query,
            }),
        ),
        RetrievalScope::Paper {
            paper_id,
            snapshot_id,
        } => (
            Some(paper_id),
            paper_id,
            serde_json::json!({
                "paperId": paper_id,
                "snapshotId": snapshot_id,
                "traceId": selection.trace_id,
                "query": selection.query,
            }),
        ),
        RetrievalScope::Manuscript { manuscript_id } => (
            None,
            "global-research",
            serde_json::json!({
                "manuscriptId": manuscript_id,
                "traceId": selection.trace_id,
                "query": selection.query,
            }),
        ),
        RetrievalScope::Note {
            resource_id,
            anchor_key,
        } => (
            None,
            resource_id,
            serde_json::json!({
                anchor_key: resource_id,
                "traceId": selection.trace_id,
                "query": selection.query,
            }),
        ),
    };
    let retrieved =
        (trace_source.load_for_scope)(connection, &selection.trace_id, scope_id, &selection.query)?;
    let source = ContextText::retrieval(
        &retrieved.trace_id,
        source_resource_id,
        &format!("trace:{}", retrieved.trace_id),
        &selection.query,
        retrieved.context_text,
        Some(anchor.to_string()),
    )
    .map_err(|_| E::Domain(RetrievalContextError::Invalid))?;
    (trace_source.mark_sent)(connection, &selection.trace_id)?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::{RetrievalContextError, RetrievalSelection};

    #[test]
    fn selection_validation_is_bounded() {
        assert!(
            RetrievalSelection {
                trace_id: "trace-1".into(),
                query: "query".into(),
            }
            .validate()
            .is_ok()
        );
        assert!(matches!(
            (RetrievalSelection {
                trace_id: "trace-1".into(),
                query: "\n".into(),
            })
            .validate(),
            Err(RetrievalContextError::Invalid)
        ));
    }
}

#[cfg(test)]
#[path = "retrieval_context_tests.rs"]
mod trace_bridge_tests;
