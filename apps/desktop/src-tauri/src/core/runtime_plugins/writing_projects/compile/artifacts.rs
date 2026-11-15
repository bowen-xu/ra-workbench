//! Original compile artifact reads, stale-input check and cancellation order.
use super::{read, repository, resources::Sql, snapshot};
use crate::{
    agent::writing_error::WritingError,
    compile_activity::CompileManager,
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{NativePreviewService, NativeResult},
        writing_projects::context::Binding,
    },
    storage::AppStore,
};
use std::{fs, sync::Arc};

pub(crate) fn current(store: &AppStore, sql: &Sql, run_id: &str) -> Result<bool, WritingError> {
    let context = {
        let db = store.connection()?;
        repository::run_input_context(sql, &db, run_id)?
    };
    let source = crate::scoped_text_files::legacy::manuscript_source_directory(
        store.manuscript_directory(),
        &context.manuscript_id,
    )?;
    Ok(snapshot::source_manifest_hash(&source)? == context.input_hash)
}

pub(crate) fn pdf(store: &AppStore, sql: &Sql, run_id: &str) -> Result<Vec<u8>, WritingError> {
    let _content_read = store.content_read()?;
    let relative_path = {
        let db = store.connection()?;
        repository::pdf_object_path(sql, &db, run_id)?
    };
    let bytes = fs::read(store.content_directory().join(relative_path))?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(WritingError::Process("编译产物不是有效 PDF".to_owned()));
    }
    Ok(bytes)
}

pub(super) fn cancel(
    store: &AppStore,
    sql: &Sql,
    manager: &CompileManager,
    run_id: &str,
) -> Result<(), WritingError> {
    {
        let db = store.connection()?;
        repository::request_cancellation(sql, &db, run_id)?;
    }
    manager.cancel(run_id)
}

#[tauri::command]
pub(crate) async fn is_runtime_manuscript_compile_current(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    run_id: String,
) -> NativeResult<bool> {
    read::task(
        store.inner().clone(),
        Binding {
            service: service.inner().clone(),
            catalog: catalog.inner().clone(),
            package_key,
        },
        move |store, domain, sql| {
            current(&store, &sql, run_id.trim()).map_err(|cause| domain.error(cause))
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn read_runtime_manuscript_compile_pdf(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    run_id: String,
) -> NativeResult<Vec<u8>> {
    read::task(
        store.inner().clone(),
        Binding {
            service: service.inner().clone(),
            catalog: catalog.inner().clone(),
            package_key,
        },
        move |store, domain, sql| {
            pdf(&store, &sql, run_id.trim()).map_err(|cause| domain.error(cause))
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn cancel_runtime_manuscript_compile(
    store: tauri::State<'_, AppStore>,
    manager: tauri::State<'_, CompileManager>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    run_id: String,
) -> NativeResult<()> {
    let manager = manager.inner().clone();
    read::task(
        store.inner().clone(),
        Binding {
            service: service.inner().clone(),
            catalog: catalog.inner().clone(),
            package_key,
        },
        move |store, domain, sql| {
            cancel(&store, &sql, &manager, run_id.trim()).map_err(|cause| domain.error(cause))
        },
    )
    .await
}
