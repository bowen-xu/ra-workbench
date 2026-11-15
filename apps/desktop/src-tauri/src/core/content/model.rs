use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentGcStatus {
    pub phase: String,
    pub is_cancellable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentStorageReport {
    pub total_object_count: i64,
    pub total_bytes: i64,
    pub current_referenced_object_count: i64,
    pub current_referenced_bytes: i64,
    pub history_only_object_count: i64,
    pub history_only_bytes: i64,
    pub pinned_history_object_count: i64,
    pub pinned_history_bytes: i64,
    pub grace_object_count: i64,
    pub grace_bytes: i64,
    pub reclaimable_object_count: i64,
    pub reclaimable_bytes: i64,
    pub available_bytes: u64,
    pub is_low_disk_space: bool,
    pub backfill_status: String,
    pub gc_status: ContentGcStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_mark_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sweep_at: Option<String>,
    pub last_success_count: i64,
    pub last_success_bytes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentGcRunReport {
    pub pruned_revision_count: usize,
    pub marked_object_count: usize,
    pub deleted_object_count: usize,
    pub deleted_bytes: u64,
    pub cleanup_warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_optional_report_fields_are_omitted_for_typescript_contracts() {
        let report = ContentStorageReport {
            total_object_count: 0,
            total_bytes: 0,
            current_referenced_object_count: 0,
            current_referenced_bytes: 0,
            history_only_object_count: 0,
            history_only_bytes: 0,
            pinned_history_object_count: 0,
            pinned_history_bytes: 0,
            grace_object_count: 0,
            grace_bytes: 0,
            reclaimable_object_count: 0,
            reclaimable_bytes: 0,
            available_bytes: 0,
            is_low_disk_space: false,
            backfill_status: "complete".to_owned(),
            gc_status: ContentGcStatus {
                phase: "idle".to_owned(),
                is_cancellable: false,
            },
            last_mark_at: None,
            last_sweep_at: None,
            last_success_count: 0,
            last_success_bytes: 0,
            last_error_code: None,
        };
        let json = serde_json::to_value(report).expect("serialize report");
        assert!(json.get("lastMarkAt").is_none());
        assert!(json.get("lastSweepAt").is_none());
        assert!(json.get("lastErrorCode").is_none());
    }
}
