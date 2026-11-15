//! Original last-PDF selection and atomic external publication.
use super::{read, repository};
use crate::{
    agent::writing_error::WritingError,
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{NativePreviewService, NativeProfile, NativeResult, package},
        writing_projects::{context::Binding, data_failure, resources::ProjectReadResources},
    },
    storage::AppStore,
    writing_transfer_files,
};
use research_workbench_native_sdk::writing_project::{Decision, Input, ManuscriptExportResult};
use std::{fs, sync::Arc};

pub(crate) async fn export_pdf(
    store: AppStore,
    binding: Binding,
    manuscript_id: String,
    destination: String,
    rename_on_conflict: bool,
) -> NativeResult<ManuscriptExportResult> {
    read::task(store, binding, move |store, domain, sql| {
        let package = package::accept(
            &domain.binding.catalog,
            &domain.binding.package_key,
            NativeProfile::WritingProjects,
        )?;
        let projects =
            ProjectReadResources::load(&domain.binding.catalog, &package).map_err(|cause| {
                crate::runtime_plugins::writing_projects::failure(
                    cause.code,
                    cause.message,
                    &domain.binding.package_key,
                )
            })?;
        let mut native_error = None;
        let result = writing_transfer_files::export_numbered(
            &destination,
            rename_on_conflict,
            |destination| {
                let content_read = store.content_read()?;
                let destination =
                    writing_transfer_files::validated_destination(destination, "pdf")?;
                writing_transfer_files::reject_managed_destination(&store, &destination)?;
                drop(content_read);
                let observation = (|| {
                    let rows = {
                        let db = store
                            .connection()
                            .map_err(|cause| domain.error(cause.into()))?;
                        projects.get(&db, &manuscript_id).map_err(data_failure)?
                    };
                    match domain.decide(Input::GetProject {
                        manuscript_id: manuscript_id.clone(),
                        rows: rows.clone(),
                    })? {
                        Decision::Project { manuscript } if manuscript.id == manuscript_id => {
                            Ok(rows)
                        }
                        Decision::Rejected { reason } => Err(domain.error(reason.into())),
                        _ => Err(domain.mismatch()),
                    }
                })();
                let rows = match observation {
                    Ok(rows) => rows,
                    Err(error) => {
                        native_error = Some(error);
                        return Err(WritingError::Process("稿件读取未完成".into()));
                    }
                };
                let _content_read = store.content_read()?;
                let source = {
                    let db = store.connection()?;
                    if let Err(cause) = projects.recheck(&db, &manuscript_id, &rows) {
                        native_error = Some(data_failure(cause));
                        return Err(WritingError::Process("稿件读取结果已变化".into()));
                    }
                    let run = repository::latest_pdf_run(&sql, &db, &manuscript_id)?
                        .ok_or(WritingError::CompileRunNotFound)?;
                    store
                        .content_directory()
                        .join(repository::pdf_object_path(&sql, &db, &run.id)?)
                };
                let bytes = fs::read(source)?;
                writing_transfer_files::export_pdf_bytes(&destination, &bytes)
            },
        );
        result.map_err(|cause| native_error.unwrap_or_else(|| domain.error(cause)))
    })
    .await
}

#[tauri::command]
pub(crate) async fn export_runtime_manuscript_pdf(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    destination_path: String,
    rename_on_conflict: Option<bool>,
) -> NativeResult<ManuscriptExportResult> {
    export_pdf(
        store.inner().clone(),
        Binding {
            service: service.inner().clone(),
            catalog: catalog.inner().clone(),
            package_key,
        },
        manuscript_id.trim().into(),
        destination_path.trim().into(),
        rename_on_conflict.unwrap_or(false),
    )
    .await
}
