use tauri::State;

use crate::storage::AppStore;

#[tauri::command]
pub fn get_docling_model_storage(
    manager: State<'_, DoclingServiceManager>,
) -> Result<super::models::ModelStorageReport, DoclingCommandError> {
    manager
        .model_resources()
        .storage_report()
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_docling_models(
    manager: State<'_, DoclingServiceManager>,
) -> Result<super::models::ModelStorageReport, DoclingCommandError> {
    manager.delete_models().await.map_err(Into::into)
}

#[tauri::command]
pub fn get_docling_model_status(
    manager: State<'_, DoclingServiceManager>,
) -> super::models::ModelStatus {
    manager.model_resources().status()
}

#[tauri::command]
pub async fn check_docling_model_update(
    store: State<'_, AppStore>,
    manager: State<'_, DoclingServiceManager>,
) -> Result<super::models::ModelStatus, DoclingCommandError> {
    let settings = {
        let connection = store.connection()?;
        repository::get_settings(&connection)?
    };
    manager
        .model_resources()
        .check(&settings.model_manifest_url)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn download_docling_models(
    store: State<'_, AppStore>,
    manager: State<'_, DoclingServiceManager>,
) -> Result<super::models::ModelStatus, DoclingCommandError> {
    let settings = {
        let connection = store.connection()?;
        repository::get_settings(&connection)?
    };
    manager
        .model_resources()
        .install(&settings.model_manifest_url)
        .await
        .map_err(Into::into)
}

use super::{
    error::{DoclingCommandError, DoclingError},
    manager::DoclingServiceManager,
    model::{
        DoclingPageOverlay, DoclingParseRunDetail, DoclingParseRunSummary, DoclingPingResult,
        DoclingServiceStatus, DoclingSettings, DoclingSettingsDraft,
    },
    repository,
};

#[tauri::command]
pub fn get_docling_settings(
    store: State<'_, AppStore>,
) -> Result<DoclingSettings, DoclingCommandError> {
    let connection = store.connection()?;
    repository::get_settings(&connection).map_err(Into::into)
}

#[tauri::command]
pub async fn save_docling_settings(
    store: State<'_, AppStore>,
    manager: State<'_, DoclingServiceManager>,
    draft: DoclingSettingsDraft,
) -> Result<DoclingSettings, DoclingCommandError> {
    let settings = {
        let connection = store.connection()?;
        repository::save_settings(&connection, &draft)?
    };
    if !settings.enabled {
        manager.disconnect().await?;
    }
    Ok(settings)
}

#[tauri::command]
pub async fn get_docling_service_status(
    manager: State<'_, DoclingServiceManager>,
) -> Result<DoclingServiceStatus, DoclingCommandError> {
    Ok(manager.refreshed_status().await)
}

#[tauri::command]
pub async fn ping_docling_service(
    manager: State<'_, DoclingServiceManager>,
) -> Result<DoclingPingResult, DoclingCommandError> {
    manager.ping().await.map_err(Into::into)
}

#[tauri::command]
pub async fn connect_docling_service(
    store: State<'_, AppStore>,
    manager: State<'_, DoclingServiceManager>,
) -> Result<DoclingServiceStatus, DoclingCommandError> {
    let settings = {
        let connection = store.connection()?;
        repository::get_settings(&connection)?
    };
    manager.start_service(&settings).await?;
    Ok(manager.refreshed_status().await)
}

#[tauri::command]
pub async fn disconnect_docling_service(
    manager: State<'_, DoclingServiceManager>,
) -> Result<DoclingServiceStatus, DoclingCommandError> {
    manager.disconnect().await?;
    Ok(manager.status())
}

#[tauri::command]
pub async fn parse_attachment_with_docling(
    store: State<'_, AppStore>,
    item_id: String,
    attachment_id: String,
    force: Option<bool>,
) -> Result<DoclingParseRunDetail, DoclingCommandError> {
    let preparation = store.docling.get().ok_or(DoclingError::Disconnected)?;
    preparation
        .prepare(&store, &item_id, &attachment_id, force.unwrap_or(false))
        .await
        .map_err(|error| DoclingCommandError::from(error.as_ref()))
}

#[tauri::command]
pub async fn list_docling_parse_runs(
    store: State<'_, AppStore>,
    item_id: String,
) -> Result<Vec<DoclingParseRunSummary>, DoclingCommandError> {
    store
        .query(move |db| repository::list_parse_runs(db, &item_id).map_err(Into::into))
        .await
}

#[tauri::command]
pub async fn get_docling_parse_run(
    store: State<'_, AppStore>,
    run_id: String,
) -> Result<DoclingParseRunDetail, DoclingCommandError> {
    store
        .query(move |db| repository::get_parse_run(db, &run_id).map_err(Into::into))
        .await
}

#[tauri::command]
pub async fn get_docling_page_overlay(
    store: State<'_, AppStore>,
    attachment_id: String,
    page_index: u32,
    parse_run_id: Option<String>,
) -> Result<DoclingPageOverlay, DoclingCommandError> {
    store
        .query(move |connection| {
            super::overlay_repository::read_page(
                connection,
                &attachment_id,
                page_index,
                parse_run_id.as_deref(),
            )
            .map_err(Into::into)
        })
        .await
}

#[tauri::command]
pub fn start_docling_parse_task(
    store: State<'_, AppStore>,
    item_id: String,
    attachment_id: String,
    force: Option<bool>,
) -> Result<super::tasks::ParseTaskStatus, DoclingCommandError> {
    super::tasks::start(
        store.inner().clone(),
        item_id,
        attachment_id,
        force.unwrap_or(false),
    )
    .map_err(Into::into)
}
#[tauri::command]
pub fn get_docling_parse_task(
    store: State<'_, AppStore>,
    task_id: String,
) -> Result<super::tasks::ParseTaskStatus, DoclingCommandError> {
    super::tasks::status(&store, &task_id, false).map_err(Into::into)
}
#[tauri::command]
pub fn cancel_docling_parse_task(
    store: State<'_, AppStore>,
    task_id: String,
) -> Result<super::tasks::ParseTaskStatus, DoclingCommandError> {
    super::tasks::status(&store, &task_id, true).map_err(Into::into)
}
