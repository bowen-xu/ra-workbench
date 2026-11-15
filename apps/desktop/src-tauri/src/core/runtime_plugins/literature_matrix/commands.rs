use super::{MatrixRequest, MatrixResult};
use crate::{
    runtime_plugins::{
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeProfile, NativeResult},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::evidence_matrix::{
    EvidenceMatrix, EvidenceMatrixCell, EvidenceMatrixCellDraft, EvidenceMatrixDraft,
};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn list_runtime_evidence_matrices(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    project_id: String,
) -> NativeResult<Vec<EvidenceMatrix>> {
    match service
        .evidence_matrix(
            store.shared_agent_connection(),
            package_key,
            MatrixRequest::List { project_id },
        )
        .await?
    {
        MatrixResult::Matrices(value) => Ok(value),
        _ => Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "矩阵结果与本次调用不匹配。",
        )),
    }
}

#[tauri::command]
pub(crate) async fn list_runtime_evidence_matrix_cells(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    matrix_id: String,
) -> NativeResult<Vec<EvidenceMatrixCell>> {
    match service
        .evidence_matrix(
            store.shared_agent_connection(),
            package_key,
            MatrixRequest::ListCells { matrix_id },
        )
        .await?
    {
        MatrixResult::Cells(value) => Ok(value),
        _ => Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "矩阵结果与本次调用不匹配。",
        )),
    }
}

#[tauri::command]
pub(crate) async fn save_runtime_evidence_matrix(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: EvidenceMatrixDraft,
) -> NativeResult<EvidenceMatrix> {
    match service
        .evidence_matrix(
            store.shared_agent_connection(),
            package_key,
            MatrixRequest::Save {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        MatrixResult::Matrix(value) => Ok(value),
        _ => Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "矩阵结果与本次调用不匹配。",
        )),
    }
}

#[tauri::command]
pub(crate) async fn save_runtime_evidence_matrix_cell(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    draft: EvidenceMatrixCellDraft,
) -> NativeResult<EvidenceMatrixCell> {
    match service
        .evidence_matrix(
            store.shared_agent_connection(),
            package_key,
            MatrixRequest::SaveCell {
                draft: Box::new(draft),
            },
        )
        .await?
    {
        MatrixResult::Cell(value) => Ok(value),
        _ => Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_response_mismatch",
            "矩阵结果与本次调用不匹配。",
        )),
    }
}
