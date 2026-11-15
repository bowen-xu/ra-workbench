//! Preserve root-file checks, then prepare and recheck the original single-row
//! autocommit metadata update. No database owner crosses a Native wait.
pub(super) mod resources;
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
use research_workbench_native_sdk::writing_project::{
    Decision, Input, Manuscript, MetadataDecision, UpdateManuscriptRequest,
};
use resources::MetadataResource;
use std::{path::PathBuf, sync::Arc};

#[tauri::command]
pub(crate) async fn update_runtime_manuscript(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    request: UpdateManuscriptRequest,
) -> NativeResult<Manuscript> {
    update_project(
        store.shared_agent_connection(),
        store.manuscript_directory().to_owned(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        request,
    )
    .await
}

pub(super) async fn update_project(
    database: SharedDatabase,
    directory: PathBuf,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    request: UpdateManuscriptRequest,
) -> NativeResult<Manuscript> {
    let selected = key.clone();
    let id = request.manuscript_id.trim().to_owned();
    let operation_id = id.clone();
    tokio::spawn(async move {
        let _operation = service.project_operation(&selected).await?;
        let accepted_key = selected.clone();
        let (reads, write) = tokio::task::spawn_blocking(move || {
            let package = package::accept(&catalog, &accepted_key, NativeProfile::WritingProjects)?;
            let reads = ProjectReadResources::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &accepted_key))?;
            let write = MetadataResource::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &accepted_key))?;
            Ok::<_, Box<NativeHostError>>((reads, write))
        })
        .await
        .map_err(|_| {
            failure(
                "plugin_host_accept_failed",
                "写作工程元数据资源核对任务未完成。",
                &selected,
            )
        })??;
        if let Some(path) = &request.root_path {
            let root = path.clone();
            let target = id.clone();
            let file_key = selected.clone();
            tokio::task::spawn_blocking(move || {
                crate::scoped_text_files::legacy::read_text_file(&directory, &target, &root)
                    .map(|_| ())
                    .map_err(|cause| {
                        let code = cause.code();
                        failure(
                            code,
                            crate::agent::writing_error::WritingError::from(cause).to_string(),
                            &file_key,
                        )
                    })
            })
            .await
            .map_err(|_| failure("storage_error", "稿件根文件读取任务未完成。", &selected))??;
            let DecidedProject {
                output,
                instance_id,
                request_id,
            } = service
                .project_decision(
                    selected.clone(),
                    Input::CheckRootKind {
                        relative_path: path.clone(),
                    },
                )
                .await?;
            match output.decision {
                Decision::RootKindAccepted { relative_path } if relative_path == *path => {}
                Decision::Rejected { reason } => {
                    return Err(decided_failure(
                        reason.code(),
                        reason.to_string(),
                        &selected,
                        &instance_id,
                        &request_id,
                    ));
                }
                _ => {
                    return Err(decided_failure(
                        "plugin_host_response_mismatch",
                        "稿件根文件种类核对未保持原路径。",
                        &selected,
                        &instance_id,
                        &request_id,
                    ));
                }
            }
        }
        let capture_db = Arc::clone(&database);
        let captured_id = id.clone();
        let captured_key = selected.clone();
        let (reads, captured) = tokio::task::spawn_blocking(move || {
            let db = capture_db
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &captured_key))?;
            let rows = reads.get(&db, &captured_id).map_err(data_failure)?;
            Ok::<_, Box<NativeHostError>>((reads, rows))
        })
        .await
        .map_err(|_| failure("storage_error", "稿件元数据读取任务未完成。", &selected))??;
        let DecidedProject {
            output,
            instance_id,
            request_id,
        } = service
            .project_decision(
                selected.clone(),
                Input::PrepareUpdate {
                    request,
                    manuscript: captured.clone(),
                },
            )
            .await?;
        drop(output.input);
        let metadata = match output.decision {
            Decision::Metadata {
                manuscript_id,
                decision: MetadataDecision::Ready { metadata },
            } if manuscript_id == id => metadata,
            Decision::Metadata {
                manuscript_id,
                decision: MetadataDecision::Rejected { reason },
            } if manuscript_id == id => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &selected,
                    &instance_id,
                    &request_id,
                ));
            }
            Decision::Rejected { reason } => {
                return Err(decided_failure(
                    reason.code(),
                    reason.to_string(),
                    &selected,
                    &instance_id,
                    &request_id,
                ));
            }
            _ => {
                return Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "稿件元数据准备未保持原身份。",
                    &selected,
                    &instance_id,
                    &request_id,
                ));
            }
        };
        let write_id = id.clone();
        let write_key = selected.clone();
        let write_instance = instance_id.clone();
        let write_request = request_id.clone();
        let (rows, changed) = tokio::task::spawn_blocking(move || {
            let mut db = database
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &write_key))?;
            reads
                .recheck(&db, &write_id, &captured)
                .map_err(data_failure)?;
            let changed = write.write(&db, &write_id, &metadata).map_err(|cause| {
                if !db.is_autocommit() {
                    db.require_recovery();
                }
                let mut error = data_failure(cause);
                error.save_outcome = Some(SaveOutcome::Unknown);
                error
            })?;
            let rows = reads.get(&db, &write_id).map_err(|cause| {
                let mut error = data_failure(cause);
                error.save_outcome = Some(if changed == 0 {
                    SaveOutcome::Unknown
                } else {
                    SaveOutcome::Committed {
                        item_id: write_id.clone(),
                    }
                });
                error
            })?;
            Ok::<_, Box<NativeHostError>>((rows, changed))
        })
        .await
        .map_err(|_| {
            let mut error = decided_failure(
                "storage_error",
                "稿件元数据更新结果未知；请按原稿件标识核对。",
                &selected,
                &instance_id,
                &request_id,
            );
            error.save_outcome = Some(SaveOutcome::Unknown);
            error
        })?
        .map_err(|mut error| {
            error.instance_id = Some(write_instance);
            error.request_id = Some(write_request);
            error
        })?;
        service
            .project_decision(
                selected.clone(),
                Input::GetProject {
                    manuscript_id: id.clone(),
                    rows,
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
            "稿件元数据更新任务未能结算；请按原稿件标识核对。",
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
