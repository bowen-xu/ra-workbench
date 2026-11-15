use crate::{
    data_interface::plugin_resources::PluginDataError,
    runtime_plugins::{literature_catalog::save::SaveOutcome, native::SafeDatabaseError},
};
use research_workbench_native_sdk::metadata::MetadataRejectionV1;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum MetadataPhaseOutcome {
    NotStarted,
    RolledBack,
    Committed,
    StatementReturned { affected_rows: u64 },
    StatementFailed,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum MetadataProblem {
    Domain(MetadataRejectionV1),
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
    Binding,
    Owner(&'static str),
    Connector(String),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataFailure {
    pub(crate) phase: &'static str,
    pub(crate) problem: Box<MetadataProblem>,
    pub(crate) outcome: MetadataPhaseOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_save: Option<SaveOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) item_id: Option<String>,
    pub(crate) completed_statements: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl MetadataFailure {
    pub(crate) fn new(
        phase: &'static str,
        problem: MetadataProblem,
        outcome: MetadataPhaseOutcome,
    ) -> Self {
        Self {
            phase,
            problem: Box::new(problem),
            outcome,
            first_save: None,
            item_id: None,
            completed_statements: Vec::new(),
            rollback_error: None,
        }
    }
}

impl std::fmt::Display for MetadataFailure {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(output, "metadata_failed ({})", self.phase)
    }
}

impl std::error::Error for MetadataFailure {}
