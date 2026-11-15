//! The existing submission owner's retained read worker; no task is installed.
use super::*;
use crate::{
    agent::repository::writing_start_inspection::Intent,
    data_interface::{SqlImmediateDiagnostic, WritingStartInspection},
};

impl WritingSubmissionService {
    pub(crate) async fn inspect_writing_start(&self, intent: Intent) -> WritingStartInspection {
        let operation = intent.request_id;
        let database = self.store.shared_agent_connection();
        self.worker_retention
            .spawn(move || database.inspect_writing_start(&intent))
            .await
            .unwrap_or_else(|_| {
                WritingStartInspection::Unconfirmed(Box::new(SqlImmediateDiagnostic {
                    operation_id: operation,
                    epoch: None,
                    sql_fence: None,
                    phase: "writing-start-status",
                    sqlite_code: None,
                    sqlite_extended_code: None,
                    safe_cause: "writing_start_inspection_worker_unavailable",
                }))
            })
    }
}
