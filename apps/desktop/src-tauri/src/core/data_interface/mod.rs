//! Application data admission. SQL callers hold the same fence as log writers.
#[path = "bootstrap.rs"]
mod bootstrap;
#[path = "database.rs"]
mod database;
#[path = "guarded_transaction.rs"]
pub(crate) mod guarded_transaction;
#[path = "journal.rs"]
pub(crate) mod journal;
#[path = "operation_identity.rs"]
pub(super) mod operation_identity;
#[path = "value_functions.rs"]
pub(super) mod value_functions;

pub(crate) mod original_sql_failure;
pub(crate) mod plugin_resources;

pub(crate) use database::sql_immediate::{
    AuthorizeRollback, CancelPreviewIntent, ChildCleanupIntent, ChildCleanupPrepared,
    ChildSubmissionInspection, ChildSubmissionIntent, ChildSubmissionPrepared,
    ContextConfigurationEnvironmentRequest, ContextConfigurationFailure,
    ContextConfigurationIntent, ContextConfigurationSeedIntent, ContextPrepareIntent,
    ContextPrepareScopeFailure, ContextPrepareSource, ContextPrepareStatus, ContextReadMode,
    ContextStartInspection, DeletionFailure, FinishSteeringHandoff, PAPER_PREPARE_FAMILY,
    PaperPrepareIntent, PaperPrepareSource, PaperPrepareStatus, PaperStartInspection,
    QUEUE_EDIT_FAMILY, QueueEditFailure, QueueEditIntent, QueueEditOutcome, QueueEditScope,
    QueueEditStatus, QueuePreviewCancelStatus, QueuePreviewPrepareIntent,
    QueuePreviewPrepareSource, QueuePreviewPrepareStatus, QueuePreviewPrepared,
    QueuePreviewRetirement, QueueStartInspection, SqlAuthorizationError, SqlImmediateDiagnostic,
    SqlImmediateFailure, SqlImmediateReceipt, SqlImmediateStatus, SqlMutationError,
    SteeringHandoffHistory, SteeringHandoffIntent, SteeringHandoffPrepared,
    SteeringHandoffResolution, TaskSettlement, WRITING_PREPARE_FAMILY, WritingPrepareIntent,
    WritingPrepareSource, WritingPrepareStatus, WritingStartInspection,
    check_context_queue_admission, has_legacy_admission, insert_context_queue_admission,
};
pub(crate) use database::{Database, DatabaseGuard, IntoDatabase, SharedDatabase};

#[derive(Debug, thiserror::Error)]
pub(crate) enum DataError {
    #[error("DATA_RUNTIME_UNAVAILABLE: data owner is unavailable")]
    Unavailable,
    #[error("DATA_RUNTIME_PAIR_INVALID: {0}")]
    Pair(&'static str),
    #[error("DATA_RUNTIME_STORAGE: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("DATA_RUNTIME_LOG: {0}")]
    Log(#[from] research_data_runtime::LogError),
    #[error("DATA_RUNTIME_ADMISSION: {0}")]
    Admission(String),
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "benchmark.rs"]
pub(crate) mod benchmark;
#[cfg(test)]
#[path = "capacity_tests.rs"]
mod capacity_tests;
