//! Original archive/restore SQL, with the same autocommit boundary and identity.
mod resources;
use super::{
    data_failure, decided_failure, failure, project_reply, resources::ProjectReadResources,
};
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_catalog::save::SaveOutcome,
        native::{
            DecidedProject, NativeHostError, NativePreviewService, NativeProfile, NativeResult,
            package,
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_project::{Decision, Input, Manuscript};
use resources::StatusResource;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) enum StatusAction {
    Archive,
    Restore,
}

#[tauri::command]
pub(crate) async fn archive_runtime_manuscript(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Manuscript> {
    change_status(
        store.shared_agent_connection(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        manuscript_id,
        StatusAction::Archive,
    )
    .await
}

#[tauri::command]
pub(crate) async fn restore_runtime_manuscript(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Manuscript> {
    change_status(
        store.shared_agent_connection(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        manuscript_id,
        StatusAction::Restore,
    )
    .await
}

pub(super) async fn change_status(
    database: SharedDatabase,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    manuscript_id: String,
    action: StatusAction,
) -> NativeResult<Manuscript> {
    let selected = key.clone();
    let id = manuscript_id.trim().to_owned();
    let operation_id = id.clone();
    tokio::spawn(async move {
        let _operation = service.project_operation(&selected).await?;
        let accepted_key = selected.clone();
        let (reads, write) = tokio::task::spawn_blocking(move || {
            let package = package::accept(&catalog, &accepted_key, NativeProfile::WritingProjects)?;
            let reads = ProjectReadResources::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &accepted_key))?;
            let write = StatusResource::load(&catalog, &package, action)
                .map_err(|error| failure(error.code, error.message, &accepted_key))?;
            Ok::<_, Box<NativeHostError>>((reads, write))
        })
        .await
        .map_err(|_| {
            failure(
                "plugin_host_accept_failed",
                "写作工程状态资源核对任务未完成。",
                &selected,
            )
        })??;
        let DecidedProject {
            output,
            instance_id: ready_instance_id,
            request_id: ready_request_id,
        } = service
            .project_decision(
                selected.clone(),
                Input::PrepareStatus {
                    manuscript_id: id.clone(),
                },
            )
            .await?;
        match output.decision {
            Decision::StatusPrepared { manuscript_id } if manuscript_id == id => {}
            Decision::Rejected { reason } => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &selected,
                    &ready_instance_id,
                    &ready_request_id,
                ));
            }
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "写作状态准备未保持原稿件身份。",
                    &selected,
                    &ready_instance_id,
                    &ready_request_id,
                ));
            }
        }
        let write_instance_id = ready_instance_id.clone();
        let write_request_id = ready_request_id.clone();
        let write_key = selected.clone();
        let target = id.clone();
        let (rows, changed) = tokio::task::spawn_blocking(move || {
            let mut db = database.lock().map_err(|cause| {
                let mut error = failure("storage_error", cause.to_string(), &write_key);
                error.instance_id = Some(write_instance_id.clone());
                error.request_id = Some(write_request_id.clone());
                error
            })?;
            let changed = write.write(&db, &target).map_err(|cause| {
                if !db.is_autocommit() {
                    db.require_recovery();
                }
                let mut error = data_failure(cause);
                error.save_outcome = Some(SaveOutcome::Unknown);
                error.instance_id = Some(write_instance_id.clone());
                error.request_id = Some(write_request_id.clone());
                error
            })?;
            // A zero count keeps the original not_found path. It cannot prove
            // that an ignored trigger produced no committed prefix elsewhere.
            let rows = if changed == 0 {
                None
            } else {
                Some(reads.get(&db, &target).map_err(|cause| {
                    let mut error = data_failure(cause);
                    error.save_outcome = Some(SaveOutcome::Committed {
                        item_id: target.clone(),
                    });
                    error.instance_id = Some(write_instance_id.clone());
                    error.request_id = Some(write_request_id.clone());
                    error
                })?)
            };
            Ok::<_, Box<NativeHostError>>((rows, changed))
        })
        .await
        .map_err(|_| {
            let mut error = failure(
                "storage_error",
                "写作工程状态结果未知；请按原稿件标识核对。",
                &selected,
            );
            error.save_outcome = Some(SaveOutcome::Unknown);
            error.instance_id = Some(ready_instance_id);
            error.request_id = Some(ready_request_id);
            error
        })??;
        service
            .project_decision(
                selected.clone(),
                Input::FinishStatus {
                    manuscript_id: id.clone(),
                    affected_rows: changed,
                    manuscript: rows,
                },
            )
            .await
            .and_then(|decided| project_reply(&selected, decided))
            .map_err(|mut error| {
                error.save_outcome = Some(if changed == 0 {
                    SaveOutcome::Unknown
                } else {
                    SaveOutcome::Committed { item_id: id }
                });
                error
            })
    })
    .await
    .map_err(|_| {
        let mut error = failure(
            "storage_error",
            "写作工程状态任务未能结算；请按原稿件标识核对。",
            &key,
        );
        error.save_outcome = Some(SaveOutcome::Unknown);
        error.operation_id = Some(operation_id.clone());
        error
    })?
    .map_err(|mut error| {
        error.operation_id = Some(operation_id);
        if error.save_outcome.is_none() {
            error.save_outcome = Some(SaveOutcome::NotStarted);
        }
        error
    })
}
