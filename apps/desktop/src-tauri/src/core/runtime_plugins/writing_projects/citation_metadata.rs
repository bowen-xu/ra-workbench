//! Finite startup metadata adapter over the original Core database owner.
mod resources;
use super::{data_failure, decided_failure, failure, resources::ProjectReadResources};
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{DecidedProject, NativePreviewService, NativeProfile, NativeResult, package},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_project::{
    BibliographyStatus, CitationBinding, CitationMetadataKind, Decision, Input,
};
use resources::Resources;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn list_runtime_manuscript_citation_bindings(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Vec<CitationBinding>> {
    match read_metadata(
        store.shared_agent_connection(),
        service.inner(),
        Arc::clone(catalog.inner()),
        package_key,
        manuscript_id,
        CitationMetadataKind::Bindings,
    )
    .await?
    {
        Decision::CitationBindings { items } => Ok(items),
        _ => unreachable!("read_metadata validates the finite response kind"),
    }
}

#[tauri::command]
pub(crate) async fn get_runtime_manuscript_bibliography_status(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<BibliographyStatus> {
    match read_metadata(
        store.shared_agent_connection(),
        service.inner(),
        Arc::clone(catalog.inner()),
        package_key,
        manuscript_id,
        CitationMetadataKind::BibliographyStatus,
    )
    .await?
    {
        Decision::BibliographyStatus { status } => Ok(status),
        _ => unreachable!("read_metadata validates the finite response kind"),
    }
}

pub(super) async fn read_metadata(
    database: SharedDatabase,
    service: &NativePreviewService,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    manuscript_id: String,
    kind: CitationMetadataKind,
) -> NativeResult<Decision> {
    let selected = key.clone();
    let (projects, metadata) = tokio::task::spawn_blocking(move || -> NativeResult<_> {
        let package = package::accept(&catalog, &selected, NativeProfile::WritingProjects)?;
        let projects = ProjectReadResources::load(&catalog, &package)
            .map_err(|error| failure(error.code, error.message, &selected))?;
        let metadata = Resources::load(&catalog, &package, kind)
            .map_err(|error| failure(error.code, error.message, &selected))?;
        Ok((projects, metadata))
    })
    .await
    .map_err(|_| {
        failure(
            "plugin_host_accept_failed",
            "写作书目资源核对任务未完成。",
            &key,
        )
    })??;
    let id = manuscript_id.trim().to_owned();
    let captured_database = Arc::clone(&database);
    let selected = key.clone();
    let captured_projects = projects.clone();
    let captured_metadata = metadata.clone();
    let input = tokio::task::spawn_blocking(move || -> NativeResult<Input> {
        let db = captured_database
            .lock()
            .map_err(|error| failure("storage_error", error.to_string(), &selected))?;
        let manuscript = captured_projects.get(&db, &id).map_err(data_failure)?;
        // The original reader resolves M before querying the related table.
        let data = if manuscript.rows.is_empty() {
            None
        } else {
            Some(captured_metadata.read(&db, &id).map_err(data_failure)?)
        };
        Ok(Input::CitationMetadata {
            metadata_kind: kind,
            manuscript_id: id,
            manuscript,
            data,
        })
    })
    .await
    .map_err(|_| failure("storage_error", "写作书目读取任务未完成。", &key))??;
    let DecidedProject {
        output,
        instance_id,
        request_id,
    } = service.project_decision(key.clone(), input.clone()).await?;
    let mismatch = || {
        decided_failure(
            "plugin_host_response_mismatch",
            "写作书目结果未保持原读取身份。",
            &key,
            &instance_id,
            &request_id,
        )
    };
    if output.input != input {
        return Err(mismatch());
    }
    let valid = match (&output.decision, kind) {
        (Decision::CitationBindings { items }, CitationMetadataKind::Bindings) => items
            .iter()
            .all(|item| item.manuscript_id == manuscript_id.trim()),
        (Decision::BibliographyStatus { .. }, CitationMetadataKind::BibliographyStatus)
        | (Decision::Rejected { .. }, _) => true,
        _ => false,
    };
    if !valid {
        return Err(mismatch());
    }
    let selected = key.clone();
    let captured_instance = instance_id.clone();
    let captured_request = request_id.clone();
    tokio::task::spawn_blocking(move || -> NativeResult<()> {
        let Input::CitationMetadata {
            manuscript_id,
            manuscript,
            data,
            ..
        } = input
        else {
            unreachable!("fixed captured input");
        };
        let checked = || -> NativeResult<()> {
            let db = database
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &selected))?;
            projects
                .recheck(&db, &manuscript_id, &manuscript)
                .map_err(data_failure)?;
            if let Some(data) = data {
                metadata
                    .recheck(&db, &manuscript_id, &data)
                    .map_err(data_failure)?;
            }
            Ok(())
        };
        checked().map_err(|mut error| {
            error.instance_id = Some(captured_instance);
            error.request_id = Some(captured_request);
            error
        })
    })
    .await
    .map_err(|_| {
        decided_failure(
            "storage_error",
            "写作书目读取核对任务未完成。",
            &key,
            &instance_id,
            &request_id,
        )
    })??;
    match output.decision {
        Decision::Rejected { reason } => Err(decided_failure(
            reason.code(),
            reason.to_string(),
            &key,
            &instance_id,
            &request_id,
        )),
        decision => Ok(decision),
    }
}
