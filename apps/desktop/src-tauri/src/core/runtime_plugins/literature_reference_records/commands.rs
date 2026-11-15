use crate::{
    runtime_plugins::{
        literature_reference_records::{ReferenceRequest, ReferenceResult},
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::reference_records::{ReferenceRecord, ReferenceRecordDraft};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn list_runtime_reference_records(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    paper_id: String,
) -> NativeResult<Vec<ReferenceRecord>> {
    match service
        .reference_records(
            store.shared_agent_connection(),
            package_key,
            ReferenceRequest::List { paper_id },
        )
        .await?
    {
        ReferenceResult::Records(records) => Ok(records),
        ReferenceResult::Record(_) => Err(mismatch()),
    }
}

#[tauri::command]
pub(crate) async fn save_runtime_reference_record(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: ReferenceRecordDraft,
) -> NativeResult<ReferenceRecord> {
    match service
        .reference_records(
            store.shared_agent_connection(),
            package_key,
            ReferenceRequest::Save {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        ReferenceResult::Record(record) => Ok(*record),
        ReferenceResult::Records(_) => Err(mismatch()),
    }
}

fn mismatch() -> Box<NativeHostError> {
    NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "参考记录结果与本次调用不匹配。",
    )
}
