use crate::runtime_plugins::native::{NativeHostError, NativeProfile};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SqlOutcome {
    NotStarted,
    RolledBack,
    Returned,
    StatementFailed,
    Unknown,
}

/// Current phase is separate from earlier commits. A failed autocommit statement
/// may retain FAIL-trigger effects; this is never an all-or-nothing batch receipt.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportJobsProgress {
    pub(crate) phase: &'static str,
    pub(crate) outcome: SqlOutcome,
    pub(crate) job_id: Option<String>,
    pub(crate) result_id: Option<String>,
    pub(crate) header_committed: Option<bool>,
    pub(crate) completed_result_updates: Option<usize>,
    pub(crate) last_save: Option<crate::runtime_plugins::literature_catalog::save::SaveOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) candidate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) candidate_insert_returned: Option<bool>,
}

impl ImportJobsProgress {
    pub(crate) fn new(phase: &'static str) -> Self {
        Self {
            phase,
            outcome: SqlOutcome::NotStarted,
            job_id: None,
            result_id: None,
            header_committed: Some(false),
            completed_result_updates: Some(0),
            last_save: None,
            candidate_id: None,
            candidate_insert_returned: None,
        }
    }

    pub(crate) fn unknown(phase: &'static str) -> Self {
        let mut progress = Self::new(phase);
        progress.outcome = SqlOutcome::Unknown;
        // The worker's local prefix is unavailable after a panic. Null means
        // unknown, rather than a claim that no earlier commit or update exists.
        progress.header_committed = None;
        progress.completed_result_updates = None;
        progress
    }

    pub(crate) fn bind(&self, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
        error.import_jobs = Some(Box::new(self.clone()));
        error
    }

    pub(crate) fn data(
        &self,
        cause: Box<crate::data_interface::plugin_resources::PluginDataError>,
    ) -> Box<NativeHostError> {
        let mut error = NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "storage_error",
            format!("文献数据库操作失败：{}", cause.safe_cause),
        );
        error.data_error = Some(cause);
        self.bind(error)
    }

    pub(crate) fn database(
        &self,
        phase: &'static str,
        cause: &rusqlite::Error,
    ) -> Box<NativeHostError> {
        let cause = super::super::native::SafeDatabaseError::sql(phase, cause);
        let mut error = NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "storage_error",
            format!("文献数据库操作失败：{cause}"),
        );
        error.save_database_error = Some(Box::new(cause));
        self.bind(error)
    }

    pub(crate) fn mismatch(&self) -> Box<NativeHostError> {
        self.bind(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "导入准备或读取结果与本次请求不匹配。",
        ))
    }
}
