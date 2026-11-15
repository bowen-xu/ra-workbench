#[path = "commands.rs"]
mod commands;
#[path = "error.rs"]
mod error;
#[path = "executable.rs"]
mod executable;
#[path = "launcher.rs"]
mod launcher;
#[path = "lease.rs"]
mod lease;
#[path = "manager.rs"]
mod manager;
#[path = "model.rs"]
mod model;
#[path = "models.rs"]
mod models;
#[path = "overlay.rs"]
mod overlay;
#[path = "overlay_repository.rs"]
mod overlay_repository;
#[path = "parse.rs"]
mod parse;
#[path = "preparation.rs"]
pub(crate) mod preparation;
#[path = "protocol.rs"]
mod protocol;
#[path = "repository.rs"]
mod repository;
#[path = "task_control.rs"]
mod task_control;
#[path = "tasks.rs"]
mod tasks;

use crate::storage::AppStore;

pub use commands::{
    cancel_docling_parse_task, check_docling_model_update, connect_docling_service,
    delete_docling_models, disconnect_docling_service, download_docling_models,
    get_docling_model_status, get_docling_model_storage, get_docling_page_overlay,
    get_docling_parse_run, get_docling_parse_task, get_docling_service_status,
    get_docling_settings, list_docling_parse_runs, parse_attachment_with_docling,
    ping_docling_service, save_docling_settings, start_docling_parse_task,
};
pub(crate) use manager::DoclingServiceManager;
use model::DoclingSettings;

pub(crate) fn schedule_startup_service(store: AppStore, manager: DoclingServiceManager) {
    tauri::async_runtime::spawn(async move {
        let settings = match tauri::async_runtime::spawn_blocking(move || {
            let connection = store.connection()?;
            repository::get_settings(&connection)
        })
        .await
        {
            Ok(Ok(settings)) => settings,
            Ok(Err(error)) => {
                eprintln!("Docling 后端自动启动配置读取失败：{error}");
                return;
            }
            Err(error) => {
                eprintln!("Docling 后端自动启动配置任务失败：{error}");
                return;
            }
        };

        if !should_start_on_app_start(&settings) {
            return;
        }

        if let Err(error) = manager.start_on_app_start(&settings).await {
            eprintln!("Docling 后端自动启动失败：{error}");
        }
    });
}

pub(crate) fn should_start_on_app_start(settings: &DoclingSettings) -> bool {
    settings.enabled
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
