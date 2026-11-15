use super::{NativeProfile, SafeDatabaseError};
use crate::agent::process::ProcessError;
use crate::hbs_storage::boundary::HbsOutcome;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageMutationDiagnostic<Outcome = HbsOutcome> {
    pub(crate) phase: &'static str,
    pub(crate) outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) object_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) database_error: Option<Box<SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

/// A failed initial signal, retained separately only after the same owner's
/// subsequent drain and input cleanup have actually confirmed completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeTerminationDiagnostic {
    operation: &'static str,
    kind: &'static str,
    raw_os_error: i32,
}

impl NativeTerminationDiagnostic {
    pub(crate) fn from_initial_group_termination(
        cause: &ProcessError,
        had_cached_exit: bool,
    ) -> Option<Self> {
        #[cfg(target_os = "macos")]
        if !had_cached_exit
            && matches!(
                cause,
                ProcessError::OperationIo {
                    operation: crate::agent::process::ProcessIoOperation::GroupTerminate,
                    kind: std::io::ErrorKind::PermissionDenied,
                    raw_os_error: Some(nix::libc::EPERM),
                }
            )
        {
            return Some(Self {
                operation: "group_terminate",
                kind: "PermissionDenied",
                raw_os_error: nix::libc::EPERM,
            });
        }
        let _ = (cause, had_cached_exit);
        None
    }
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code}: {message}")]
pub(crate) struct NativeHostError {
    pub code: String,
    pub message: String,
    pub plugin_id: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub termination_error: Option<NativeTerminationDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_error: Option<Box<crate::data_interface::plugin_resources::PluginDataError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_outcome: Option<crate::runtime_plugins::literature_catalog::save::SaveOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_database_error:
        Option<Box<crate::runtime_plugins::literature_catalog::save::SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_rollback_error:
        Option<Box<crate::runtime_plugins::literature_catalog::save::SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_outcome: Option<crate::runtime_plugins::literature_nal_query::QueryOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_database_error: Option<Box<super::SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_rollback_error: Option<Box<super::SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_error_kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_expected_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap_outcome:
        Option<crate::runtime_plugins::literature_nal_bootstrap::BootstrapOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap_problem:
        Option<Box<crate::runtime_plugins::literature_nal_bootstrap::BootstrapProblem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap_rollback_error: Option<Box<super::SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_outcome:
        Option<crate::runtime_plugins::literature_nal_annotation::AnnotationOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_problem:
        Option<Box<crate::runtime_plugins::literature_nal_annotation::AnnotationProblem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_rollback_error: Option<Box<super::SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_outcome: Option<crate::runtime_plugins::literature_nal_mutation::MutationOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_problem:
        Option<Box<crate::runtime_plugins::literature_nal_mutation::MutationProblem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_rollback_error: Option<Box<super::SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_sql: Option<crate::runtime_plugins::literature_duplicates::DecisionSqlState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_database_error: Option<Box<super::SafeDatabaseError>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_undo: Option<Box<crate::runtime_plugins::literature_duplicate_undo::UndoFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_merge:
        Option<Box<crate::runtime_plugins::literature_duplicate_merge::MergeFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literature_lifecycle:
        Option<Box<crate::runtime_plugins::literature_lifecycle::LifecycleFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trash_cleanup:
        Option<Box<crate::runtime_plugins::literature_trash_cleanup::TrashCleanupFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literature_metadata:
        Option<Box<crate::runtime_plugins::literature_metadata::MetadataFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_jobs:
        Option<Box<crate::runtime_plugins::literature_import_jobs::ImportJobsProgress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_write:
        Option<Box<crate::runtime_plugins::literature_claims::EvidenceWriteDiagnostic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_view: Option<Box<StorageMutationDiagnostic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relations: Option<Box<StorageMutationDiagnostic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept_identity: Option<Box<StorageMutationDiagnostic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept_algorithm: Option<Box<StorageMutationDiagnostic<StorageMutationOutcome>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_matrix: Option<Box<crate::runtime_plugins::literature_matrix::MatrixDiagnostic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_records:
        Option<Box<crate::runtime_plugins::literature_reference_records::ReferenceFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Box<crate::runtime_plugins::literature_attachments::AttachmentFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concepts: Option<Box<crate::runtime_plugins::literature_concepts::ConceptFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_preparation:
        Option<Box<crate::runtime_plugins::literature_context::preparation::PreparationDiagnostic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading_units:
        Option<Box<crate::runtime_plugins::literature_reading_units::ReadingUnitFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_confirmation: Option<Box<StorageMutationDiagnostic>>,
}

pub(crate) type NativeResult<T> = Result<T, Box<NativeHostError>>;

impl NativeHostError {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Box<Self> {
        Self::for_profile(NativeProfile::WritingPreview, code, message)
    }
    pub(crate) fn for_profile(
        profile: NativeProfile,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Box<Self> {
        Box::new(Self {
            code: code.into(),
            message: message.into(),
            plugin_id: profile.plugin_id(),
            package_key: None,
            instance_id: None,
            request_id: None,
            operation_id: None,
            exit_code: None,
            signal: None,
            cleanup_error: None,
            termination_error: None,
            data_error: None,
            save_outcome: None,
            save_database_error: None,
            save_rollback_error: None,
            query_outcome: None,
            query_database_error: None,
            query_rollback_error: None,
            query_error_kind: None,
            query_expected_revision: None,
            bootstrap_outcome: None,
            bootstrap_problem: None,
            bootstrap_rollback_error: None,
            annotation_outcome: None,
            annotation_problem: None,
            annotation_rollback_error: None,
            mutation_outcome: None,
            mutation_problem: None,
            mutation_rollback_error: None,
            duplicate_sql: None,
            duplicate_database_error: None,
            duplicate_undo: None,
            duplicate_merge: None,
            literature_lifecycle: None,
            trash_cleanup: None,
            literature_metadata: None,
            import_jobs: None,
            reference_records: None,
            evidence_matrix: None,
            derived_view: None,
            relations: None,
            concept_identity: None,
            concept_algorithm: None,
            evidence_write: None,
            attachments: None,
            concepts: None,
            current_revision: None,
            paper_preparation: None,
            reading_units: None,
            paper_confirmation: None,
        })
    }
    pub(crate) fn package(mut self: Box<Self>, key: &str) -> Box<Self> {
        self.package_key = Some(key.into());
        self
    }
    pub(crate) fn add_cleanup(&mut self, cause: impl Into<String>) {
        let cause = cause.into();
        match &mut self.cleanup_error {
            Some(previous) => {
                previous.push_str("; ");
                previous.push_str(&cause);
            }
            None => self.cleanup_error = Some(cause),
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StorageMutationOutcome {
    NotStarted,
    RolledBack,
    Committed,
    Unknown,
    StatementFailed,
}
impl From<HbsOutcome> for StorageMutationOutcome {
    fn from(value: HbsOutcome) -> Self {
        match value {
            HbsOutcome::NotStarted => Self::NotStarted,
            HbsOutcome::RolledBack => Self::RolledBack,
            HbsOutcome::Committed => Self::Committed,
            HbsOutcome::Unknown => Self::Unknown,
        }
    }
}
