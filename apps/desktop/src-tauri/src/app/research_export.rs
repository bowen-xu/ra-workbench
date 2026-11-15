//! Original export commands select one native package or the same static rules.
pub(crate) use crate::runtime_plugins::literature_export::ResearchExportManager;
use crate::{
    runtime_plugins::{
        literature_export::{
            ResearchExportCommandError, ResearchExportError, ResearchExportReport,
            ResearchExportRequest,
        },
        literature_trace::service::LiteratureTraceService,
        native::{NativeHostError, NativeResult},
    },
    storage::AppStore,
};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum ResearchExportRouteError {
    Native(Box<NativeHostError>),
    Export(ResearchExportCommandError),
}
impl From<Box<NativeHostError>> for ResearchExportRouteError {
    fn from(error: Box<NativeHostError>) -> Self {
        Self::Native(error)
    }
}
impl From<ResearchExportError> for ResearchExportRouteError {
    fn from(error: ResearchExportError) -> Self {
        Self::Export(error.into())
    }
}

#[tauri::command]
pub async fn export_research_knowledge(
    store: State<'_, AppStore>,
    manager: State<'_, ResearchExportManager>,
    request: ResearchExportRequest,
) -> Result<ResearchExportReport, ResearchExportRouteError> {
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .research_export(
                store.inner().clone(),
                binding.package_key,
                manager.inner().clone(),
                request.target_directory,
            )
            .await
            .map_err(Into::into);
    }

    {
        let _ = request;
        Err(unavailable())
    }
}

#[tauri::command]
pub fn cancel_research_export(manager: State<'_, ResearchExportManager>) -> bool {
    manager.cancel()
}

#[tauri::command]
pub(crate) async fn export_runtime_research_knowledge(
    store: State<'_, AppStore>,
    service: State<'_, Arc<LiteratureTraceService>>,
    manager: State<'_, ResearchExportManager>,
    package_key: String,
    request: ResearchExportRequest,
) -> NativeResult<ResearchExportReport> {
    service
        .research_export(
            store.inner().clone(),
            package_key,
            manager.inner().clone(),
            request.target_directory,
        )
        .await
}
#[tauri::command]
pub(crate) async fn cancel_runtime_research_export(
    service: State<'_, Arc<LiteratureTraceService>>,
    manager: State<'_, ResearchExportManager>,
    package_key: String,
) -> NativeResult<bool> {
    service.cancel_research_export(&package_key, &manager).await
}

fn unavailable() -> ResearchExportRouteError {
    NativeHostError::for_profile(
        crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
        "capability_unavailable",
        "literature.research-export 不可用。",
    )
    .into()
}
