//! Original recovery storage over owned Native preparation and projections.
mod resources;
use super::{data_failure, decided_failure, failure, resources::ProjectReadResources};
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
    Decision, Input, RecoveryDraft, RecoveryDraftInput,
};
use resources::RecoveryResources;
use std::sync::Arc;

async fn resources(
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
) -> NativeResult<(ProjectReadResources, RecoveryResources)> {
    let selected = key.clone();
    tokio::task::spawn_blocking(move || {
        let package = package::accept(&catalog, &selected, NativeProfile::WritingProjects)?;
        let reads = ProjectReadResources::load(&catalog, &package)
            .map_err(|error| failure(error.code, error.message, &selected))?;
        let recovery = RecoveryResources::load(&catalog, &package)
            .map_err(|error| failure(error.code, error.message, &selected))?;
        Ok((reads, recovery))
    })
    .await
    .map_err(|_| {
        failure(
            "plugin_host_accept_failed",
            "写作恢复草稿资源核对任务未完成。",
            &key,
        )
    })?
}

#[tauri::command]
pub(crate) async fn list_runtime_manuscript_recovery_drafts(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Vec<RecoveryDraft>> {
    list_drafts(
        store.shared_agent_connection(),
        service.inner(),
        Arc::clone(catalog.inner()),
        package_key,
        manuscript_id,
    )
    .await
}

pub(super) async fn list_drafts(
    database: SharedDatabase,
    service: &NativePreviewService,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    manuscript_id: String,
) -> NativeResult<Vec<RecoveryDraft>> {
    let (reads, recovery) = resources(catalog, key.clone()).await?;
    let id = manuscript_id.trim().to_owned();
    let selected = key.clone();
    let input = tokio::task::spawn_blocking(move || -> NativeResult<Input> {
        let db = database
            .lock()
            .map_err(|error| failure("storage_error", error.to_string(), &selected))?;
        let manuscript = reads.get(&db, &id).map_err(data_failure)?;
        let drafts = if manuscript.rows.is_empty() {
            None
        } else {
            Some(recovery.list(&db, &id).map_err(data_failure)?)
        };
        Ok(Input::ListRecoveryDrafts {
            manuscript_id: id,
            manuscript,
            drafts,
        })
    })
    .await
    .map_err(|_| failure("storage_error", "写作恢复草稿读取任务未完成。", &key))??;
    let DecidedProject {
        output,
        instance_id,
        request_id,
    } = service.project_decision(key.clone(), input).await?;
    match output.decision {
        Decision::RecoveryDrafts { drafts }
            if drafts
                .iter()
                .all(|draft| draft.manuscript_id == manuscript_id.trim()) =>
        {
            Ok(drafts)
        }
        Decision::Rejected { reason } => Err(decided_failure(
            reason.code(),
            reason.to_string(),
            &key,
            &instance_id,
            &request_id,
        )),
        _ => Err(decided_failure(
            "plugin_host_response_mismatch",
            "写作恢复草稿列表未保持原稿件身份。",
            &key,
            &instance_id,
            &request_id,
        )),
    }
}

#[tauri::command]
pub(crate) async fn save_runtime_manuscript_recovery_draft(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    draft: RecoveryDraftInput,
) -> NativeResult<RecoveryDraft> {
    save_draft(
        store.shared_agent_connection(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        draft,
    )
    .await
}

pub(super) async fn save_draft(
    database: SharedDatabase,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    draft: RecoveryDraftInput,
) -> NativeResult<RecoveryDraft> {
    let selected = key.clone();
    let id = draft.manuscript_id.trim().to_owned();
    let operation_id = id.clone();
    tokio::spawn(async move {
        let _operation = service.project_operation(&selected).await?;
        let (reads, recovery) = resources(catalog, selected.clone()).await?;
        let captured_db = Arc::clone(&database);
        let captured_id = id.clone();
        let captured_key = selected.clone();
        let (reads, captured) = tokio::task::spawn_blocking(move || {
            let db = captured_db
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &captured_key))?;
            let rows = reads.get(&db, &captured_id).map_err(data_failure)?;
            Ok::<_, Box<NativeHostError>>((reads, rows))
        })
        .await
        .map_err(|_| {
            failure(
                "storage_error",
                "写作恢复草稿稿件核对任务未完成。",
                &selected,
            )
        })??;
        let DecidedProject {
            output,
            instance_id,
            request_id,
        } = service
            .project_decision(
                selected.clone(),
                Input::PrepareRecoveryDraft {
                    draft: draft.clone(),
                    manuscript: captured.clone(),
                },
            )
            .await?;
        let retention = match output.decision {
            Decision::RecoveryDraftPrepared {
                draft: prepared,
                retention,
            } if prepared == draft => retention,
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
                    "写作恢复草稿准备未保持原输入。",
                    &selected,
                    &instance_id,
                    &request_id,
                ));
            }
        };
        drop(output.input);
        let limits = i64::try_from(retention.per_manuscript)
            .and_then(|per| i64::try_from(retention.total).map(|total| (per, total)))
            .map_err(|_| {
                decided_failure(
                    "plugin_host_response_mismatch",
                    "写作恢复草稿保留上限不兼容原SQL整数。",
                    &selected,
                    &instance_id,
                    &request_id,
                )
            })?;
        let write_key = selected.clone();
        let write_id = id.clone();
        let path = draft.relative_path.clone();
        let saved_instance = instance_id.clone();
        let saved_request = request_id.clone();
        let rows = tokio::task::spawn_blocking(move || {
            let mut db = database
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &write_key))?;
            reads
                .recheck(&db, &write_id, &captured)
                .map_err(data_failure)?;
            // Preserve raw draft IDs, paths and bytes at the original SQL call.
            // The earlier existence lookup alone trims M, as the original did.
            recovery
                .save(&db, &draft, limits.0, limits.1)
                .map_err(|cause| {
                    if !db.is_autocommit() {
                        db.require_recovery();
                    }
                    let mut error = data_failure(cause);
                    error.save_outcome = Some(SaveOutcome::Unknown);
                    error
                })
        })
        .await
        .map_err(|_| {
            let mut error = decided_failure(
                "storage_error",
                "写作恢复草稿保存结果未知；请读取原草稿核对。",
                &selected,
                &instance_id,
                &request_id,
            );
            error.save_outcome = Some(SaveOutcome::Unknown);
            error
        })?
        .map_err(|mut error| {
            error.instance_id = Some(saved_instance);
            error.request_id = Some(saved_request);
            error
        })?;
        let result = service
            .project_decision(
                selected.clone(),
                Input::ProjectRecoveryDraft {
                    manuscript_id: id.clone(),
                    relative_path: path.clone(),
                    rows,
                },
            )
            .await;
        let expected_id = id.clone();
        let outcome = SaveOutcome::Committed { item_id: id };
        let result = result.and_then(|decided| {
            let DecidedProject {
                output,
                instance_id,
                request_id,
            } = decided;
            match output.decision {
                Decision::RecoveryDraft { draft }
                    if draft.manuscript_id == expected_id && draft.relative_path == path =>
                {
                    Ok(draft)
                }
                _ => Err(decided_failure(
                    "plugin_host_response_mismatch",
                    "写作恢复草稿读回未保持原稿件与路径。",
                    &selected,
                    &instance_id,
                    &request_id,
                )),
            }
        });
        result.map_err(|mut error| {
            error.save_outcome = Some(outcome);
            error
        })
    })
    .await
    .map_err(|_| {
        let mut error = failure(
            "storage_error",
            "写作恢复草稿保存任务未能结算；请读取原草稿核对。",
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

#[tauri::command]
pub(crate) async fn clear_runtime_manuscript_recovery_draft(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
    relative_path: String,
) -> NativeResult<()> {
    clear_draft(
        store.shared_agent_connection(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        manuscript_id,
        relative_path,
    )
    .await
}

pub(super) async fn clear_draft(
    database: SharedDatabase,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    manuscript_id: String,
    relative_path: String,
) -> NativeResult<()> {
    let selected = key.clone();
    let id = manuscript_id.trim().to_owned();
    let path = relative_path.trim().to_owned();
    let operation_id = id.clone();
    tokio::spawn(async move {
        let _operation = service.project_operation(&selected).await?;
        let (_, recovery) = resources(catalog, selected.clone()).await?;
        let probe_db = Arc::clone(&database);
        let probe_key = selected.clone();
        tokio::task::spawn_blocking(move || -> NativeResult<()> {
            drop(
                probe_db
                    .lock()
                    .map_err(|error| failure("storage_error", error.to_string(), &probe_key))?,
            );
            Ok(())
        })
        .await
        .map_err(|_| {
            failure(
                "storage_error",
                "写作恢复草稿清除存储核对任务未完成。",
                &selected,
            )
        })??;
        let DecidedProject {
            output,
            instance_id,
            request_id,
        } = service
            .project_decision(
                selected.clone(),
                Input::PrepareRecoveryClear {
                    manuscript_id: id.clone(),
                    relative_path: path.clone(),
                },
            )
            .await?;
        match output.decision {
            Decision::RecoveryClearPrepared {
                manuscript_id,
                relative_path,
            } if manuscript_id == id && relative_path == path => {}
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
                    "写作恢复草稿清除准备未保持原稿件与路径。",
                    &selected,
                    &instance_id,
                    &request_id,
                ));
            }
        }
        let write_key = selected.clone();
        let saved_instance = instance_id.clone();
        let saved_request = request_id.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = database
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &write_key))?;
            recovery.clear(&db, &id, &path).map_err(|cause| {
                if !db.is_autocommit() {
                    db.require_recovery();
                }
                let mut error = data_failure(cause);
                error.save_outcome = Some(SaveOutcome::Unknown);
                error
            })
        })
        .await
        .map_err(|_| {
            let mut error = decided_failure(
                "storage_error",
                "写作恢复草稿清除结果未知；请读取原草稿核对。",
                &selected,
                &instance_id,
                &request_id,
            );
            error.save_outcome = Some(SaveOutcome::Unknown);
            error
        })?
        .map_err(|mut error| {
            error.instance_id = Some(saved_instance);
            error.request_id = Some(saved_request);
            error
        })
    })
    .await
    .map_err(|_| {
        let mut error = failure(
            "storage_error",
            "写作恢复草稿清除任务未能结算；请读取原草稿核对。",
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
