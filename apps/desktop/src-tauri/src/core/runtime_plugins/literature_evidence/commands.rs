use super::{EvidenceReadRequest, EvidenceReadResult};
use crate::{
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::evidence::{Claim, EvidenceLink, SourceAnchorSummary};
use std::sync::Arc;
#[tauri::command]
pub(crate) async fn list_runtime_evidence_claims(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    project_id: Option<String>,
) -> NativeResult<Vec<Claim>> {
    match service
        .evidence_read(
            store.shared_agent_connection(),
            package_key,
            EvidenceReadRequest::Claims { project_id },
        )
        .await?
    {
        EvidenceReadResult::Claims(value) => Ok(value),
        _ => Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "证据读取结果不匹配。",
        )),
    }
}
#[tauri::command]
pub(crate) async fn list_runtime_evidence_sources(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    project_id: Option<String>,
) -> NativeResult<Vec<SourceAnchorSummary>> {
    match service
        .evidence_read(
            store.shared_agent_connection(),
            package_key,
            EvidenceReadRequest::Sources { project_id },
        )
        .await?
    {
        EvidenceReadResult::Sources(value) => Ok(value),
        _ => Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "证据读取结果不匹配。",
        )),
    }
}
#[tauri::command]
pub(crate) async fn list_runtime_claim_evidence(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    claim_id: String,
) -> NativeResult<Vec<EvidenceLink>> {
    match service
        .evidence_read(
            store.shared_agent_connection(),
            package_key,
            EvidenceReadRequest::Links { claim_id },
        )
        .await?
    {
        EvidenceReadResult::Links(value) => Ok(value),
        _ => Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "证据读取结果不匹配。",
        )),
    }
}
