use super::{ClaimsRequest, ClaimsResult};
use crate::{
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::evidence_write::*;
use std::sync::Arc;
#[tauri::command]
pub(crate) async fn save_runtime_evidence_claim(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: ClaimDraft,
) -> NativeResult<Claim> {
    match service
        .evidence_write(
            store.shared_agent_connection(),
            package_key,
            ClaimsRequest::Save {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        ClaimsResult::Claim(value) => Ok(*value),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn link_runtime_claim_to_anchor(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: EvidenceLinkDraft,
) -> NativeResult<EvidenceLink> {
    match service
        .evidence_write(
            store.shared_agent_connection(),
            package_key,
            ClaimsRequest::Link {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        ClaimsResult::Link(value) => Ok(*value),
        _ => Err(mismatch()),
    }
}
#[tauri::command]
pub(crate) async fn create_runtime_claim_with_evidence(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    claim: ClaimDraft,
    evidence: NewEvidenceDraft,
) -> NativeResult<Claim> {
    match service
        .evidence_write(
            store.shared_agent_connection(),
            package_key,
            ClaimsRequest::Create {
                claim: Box::new(claim),
                evidence,
            },
        )
        .await?
    {
        ClaimsResult::Claim(value) => Ok(*value),
        _ => Err(mismatch()),
    }
}
fn mismatch() -> Box<NativeHostError> {
    NativeHostError::for_profile(
        NativeProfile::NoteTraceDecision,
        "plugin_host_response_mismatch",
        "人工观点与证据结果与本次调用不匹配。",
    )
}
