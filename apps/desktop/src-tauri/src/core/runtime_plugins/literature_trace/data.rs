//! Note Page and paper trace consumption inside the original caller transaction.
//!
//! The caller retains binding, Note/selection validation and shared admission.
//! This adapter owns no connection, transaction, process or domain trace policy.
use crate::{
    agent::retrieval_context::RetrievalSelection,
    data_interface::plugin_resources::{PluginDataError, PreparedDataRead},
    runtime_plugins::{
        RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    },
};
use research_agent_core::submission::ContextText;
use research_workbench_native_sdk::{
    data::{DataAccess, DataCall, DataField, DataResourceGrant, DataRows, DataType, DataValue},
    literature_trace_protocol::{SnapshotInput, TraceDecision},
    paper_retrieval::PaperRetrievalInputV1,
    retrieval_trace::{MarkSentCondition, RetrievalTraceRow, TraceRejection},
};
use rusqlite::Transaction;

const OWNER: &str = "literature_library";
const READ_HASH: &str = "3dc4391e2def0bbaf1318012d8e66ba6a1007e680f12d961d82f9cb8402da016";
const EXPIRE_HASH: &str = "f9558d513ed49347de972312fd1562453b39d46397ec20c96578c67dd0173424";
const MARK_HASH: &str = "bf5ab9a69362afebd1eb5c27669d2fdbcca17acf7b2f21c6abfc06dc5fb672b4";

pub(crate) struct TraceResources {
    read: AcceptedDataResource,
    expire: AcceptedDataResource,
    mark_sent: AcceptedDataResource,
}

impl TraceResources {
    /// Grants are fixed Core metadata; SQL bytes are read only from this package.
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |name: &str, hash: &str, access: DataAccess, columns: Vec<DataField>| {
            AcceptedDataResource::load(
                catalog,
                package,
                OWNER,
                &format!("data/trace-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.retrieval.trace-{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters: vec![text(false)],
                    columns,
                },
            )
        };
        Ok(Self {
            read: load(
                "read",
                READ_HASH,
                DataAccess::Read,
                vec![
                    text(true),
                    text(false),
                    text(false),
                    text(false),
                    text(false),
                    DataField {
                        data_type: DataType::Integer,
                        nullable: false,
                    },
                ],
            )?,
            expire: load("expire", EXPIRE_HASH, DataAccess::Write, vec![])?,
            mark_sent: load("mark-sent", MARK_HASH, DataAccess::Write, vec![])?,
        })
    }
}

/// Only safe categories and the existing SQL diagnostics cross this boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TraceDataError {
    #[error(transparent)]
    Data(#[from] Box<PluginDataError>),
    #[error("retrieval trace observation has an incompatible shape")]
    InvalidObservation,
    #[error("retrieval trace decision has an incompatible postcondition")]
    InvalidDecision,
    #[error("retrieval selection is invalid")]
    InvalidSource,
    #[error("retrieval trace is unavailable or no longer matches its scope")]
    Rejected(TraceRejection),
}

enum TraceScope {
    NotePage(String),
    Paper {
        paper_id: String,
        snapshot_id: String,
    },
}

/// Complete owned observations. Scope and source anchors remain Core-owned.
pub(crate) struct PreparedTrace {
    read: PreparedDataRead,
    resources: TraceResources,
    selection: RetrievalSelection,
    scope: TraceScope,
    row: Option<RetrievalTraceRow>,
}
impl std::fmt::Debug for PreparedTrace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print query, retrieved research text or captured SQL observations.
        formatter
            .debug_struct("PreparedTrace")
            .finish_non_exhaustive()
    }
}
impl PreparedTrace {
    pub(crate) fn note_input(&self) -> Result<SnapshotInput, TraceDataError> {
        if !matches!(self.scope, TraceScope::NotePage(_)) {
            return Err(TraceDataError::InvalidObservation);
        }
        Ok(SnapshotInput {
            query: self.selection.query.clone(),
            row: self.row.clone(),
        })
    }
    pub(crate) fn paper_input(&self) -> Result<PaperRetrievalInputV1, TraceDataError> {
        let TraceScope::Paper { paper_id, .. } = &self.scope else {
            return Err(TraceDataError::InvalidObservation);
        };
        Ok(PaperRetrievalInputV1 {
            paper_id: paper_id.clone(),
            query: self.selection.query.clone(),
            row: self.row.clone(),
        })
    }
    pub(crate) fn matches_paper(
        &self,
        paper: &str,
        snapshot: &str,
        selection: &RetrievalSelection,
    ) -> bool {
        matches!(&self.scope, TraceScope::Paper {paper_id,snapshot_id} if paper_id==paper && snapshot_id==snapshot)
            && self.selection.trace_id == selection.trace_id
            && self.selection.query == selection.query
    }
}

/// Call after the original binding, Note materialization and selection checks.
pub(crate) fn capture_note_trace_in(
    tx: &Transaction<'_>,
    resources: TraceResources,
    selection: &RetrievalSelection,
    page_id: &str,
) -> Result<PreparedTrace, TraceDataError> {
    capture_trace_in(
        tx,
        resources,
        selection,
        TraceScope::NotePage(page_id.into()),
    )
}
pub(crate) fn capture_paper_trace_in(
    tx: &Transaction<'_>,
    resources: TraceResources,
    selection: &RetrievalSelection,
    paper_id: &str,
    snapshot_id: &str,
) -> Result<PreparedTrace, TraceDataError> {
    capture_trace_in(
        tx,
        resources,
        selection,
        TraceScope::Paper {
            paper_id: paper_id.into(),
            snapshot_id: snapshot_id.into(),
        },
    )
}
fn capture_trace_in(
    tx: &Transaction<'_>,
    resources: TraceResources,
    selection: &RetrievalSelection,
    scope: TraceScope,
) -> Result<PreparedTrace, TraceDataError> {
    let read = PreparedDataRead::capture(
        tx,
        resources.read.clone(),
        call(&resources.read, &selection.trace_id),
    )?;
    let row = decode_row(read.rows())?;
    Ok(PreparedTrace {
        read,
        resources,
        selection: selection.clone(),
        scope,
        row,
    })
}

/// Invoke at the original trace-consumption position, after the caller repeats
/// its Core prechecks. The caller alone commits/rolls back all subsequent writes.
pub(crate) fn consume_prepared_trace_in(
    tx: &Transaction<'_>,
    prepared: &PreparedTrace,
    decision: &TraceDecision,
) -> Result<ContextText, TraceDataError> {
    let checked = prepared.read.recheck_in(tx)?;
    match decision {
        TraceDecision::Rejected { reason } => {
            if *reason == TraceRejection::Expired {
                // Preserve the original SQL-error-before-Expired ordering and
                // do not introduce an affected-row postcondition for expire.
                checked.execute(
                    &prepared.resources.expire,
                    &call(&prepared.resources.expire, &prepared.selection.trace_id),
                )?;
            }
            Err(TraceDataError::Rejected(*reason))
        }
        TraceDecision::Ready { mark_sent } => {
            if *mark_sent != MarkSentCondition::ORIGINAL {
                return Err(TraceDataError::InvalidDecision);
            }
            let row = prepared
                .row
                .as_ref()
                .ok_or(TraceDataError::InvalidDecision)?;
            let selection = &prepared.selection;
            let (resource, anchor) = match &prepared.scope {
                TraceScope::NotePage(page_id) => (
                    page_id.as_str(),
                    serde_json::json!({
                        "pageId":page_id,"traceId":selection.trace_id,"query":selection.query,
                    }),
                ),
                TraceScope::Paper {
                    paper_id,
                    snapshot_id,
                } => (
                    paper_id.as_str(),
                    serde_json::json!({
                        "paperId":paper_id,"snapshotId":snapshot_id,"traceId":selection.trace_id,"query":selection.query,
                    }),
                ),
            };
            // This Core factory must fail before any mark-sent mutation.
            let source = ContextText::retrieval(
                &selection.trace_id,
                resource,
                &format!("trace:{}", selection.trace_id),
                &selection.query,
                row.context_text.clone(),
                Some(anchor.to_string()),
            )
            .map_err(|_| TraceDataError::InvalidSource)?;
            let changed = checked.execute(
                &prepared.resources.mark_sent,
                &call(&prepared.resources.mark_sent, &selection.trace_id),
            )?;
            if changed != mark_sent.expected_rows {
                return Err(TraceDataError::Rejected(mark_sent.on_mismatch));
            }
            Ok(source)
        }
    }
}

fn text(nullable: bool) -> DataField {
    DataField {
        data_type: DataType::Text,
        nullable,
    }
}

fn call(resource: &AcceptedDataResource, trace_id: &str) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments: vec![DataValue::Text(trace_id.into())],
    }
}

fn decode_row(rows: &DataRows) -> Result<Option<RetrievalTraceRow>, TraceDataError> {
    let row = match rows.rows.as_slice() {
        [] => return Ok(None),
        [row] => row,
        _ => return Err(TraceDataError::InvalidObservation),
    };
    match row.as_slice() {
        [
            paper_id,
            DataValue::Text(query_sha256),
            DataValue::Text(status),
            DataValue::Text(context_text),
            DataValue::Text(context_sha256),
            DataValue::Integer(expired),
        ] => {
            let paper_id = match paper_id {
                DataValue::Null => None,
                DataValue::Text(value) => Some(value.clone()),
                _ => return Err(TraceDataError::InvalidObservation),
            };
            Ok(Some(RetrievalTraceRow {
                paper_id,
                query_sha256: query_sha256.clone(),
                status: status.clone(),
                context_text: context_text.clone(),
                context_sha256: context_sha256.clone(),
                // Same conversion as the original rusqlite row.get::<_,bool>.
                expired: *expired != 0,
            }))
        }
        _ => Err(TraceDataError::InvalidObservation),
    }
}

#[cfg(test)]
mod tests;
