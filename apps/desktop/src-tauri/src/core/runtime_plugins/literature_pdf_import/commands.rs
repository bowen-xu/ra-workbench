use crate::{
    runtime_plugins::{literature_trace::service::LiteratureTraceService, native::NativeResult},
    storage::AppStore,
};
use research_workbench_native_sdk::{import_jobs::ImportedJob, pdf_import::PdfImportCandidate};
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn start_runtime_pdf_import(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<LiteratureTraceService>>,
    package_key: String,
    candidates: Vec<PdfImportCandidate>,
) -> NativeResult<ImportedJob> {
    service
        .start_pdf_import(store.inner().clone(), package_key, candidates)
        .await
}
