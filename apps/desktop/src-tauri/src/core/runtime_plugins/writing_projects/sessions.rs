//! Original session persistence over a finite Native decision and fixed SQL.
use super::{
    data_failure, decided_failure, failure,
    resources::{ProjectReadResources, SessionResources},
};
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{DecidedProject, NativePreviewService, NativeProfile, NativeResult, package},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_project::{
    Decision, Input, WritingSession, WritingSessionDraft,
};
use std::sync::Arc;

async fn resources(
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
) -> NativeResult<(ProjectReadResources, SessionResources)> {
    let selected = key.clone();
    tokio::task::spawn_blocking(move || {
        let package = package::accept(&catalog, &selected, NativeProfile::WritingProjects)?;
        let reads = ProjectReadResources::load(&catalog, &package)
            .map_err(|error| failure(error.code, error.message, &selected))?;
        let sessions = SessionResources::load(&catalog, &package)
            .map_err(|error| failure(error.code, error.message, &selected))?;
        Ok((reads, sessions))
    })
    .await
    .map_err(|_| {
        failure(
            "plugin_host_accept_failed",
            "写作会话资源核对任务未完成。",
            &key,
        )
    })?
}

#[tauri::command]
pub(crate) async fn get_runtime_writing_session(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Option<WritingSession>> {
    get_session(
        store.shared_agent_connection(),
        service.inner(),
        Arc::clone(catalog.inner()),
        package_key,
        manuscript_id,
    )
    .await
}

pub(super) async fn get_session(
    database: SharedDatabase,
    service: &NativePreviewService,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    manuscript_id: String,
) -> NativeResult<Option<WritingSession>> {
    let (reads, sessions) = resources(catalog, key.clone()).await?;
    let id = manuscript_id.trim().to_owned();
    let selected = key.clone();
    let input = tokio::task::spawn_blocking(move || -> NativeResult<Input> {
        let db = database
            .lock()
            .map_err(|error| failure("storage_error", error.to_string(), &selected))?;
        let manuscript = reads.get(&db, &id).map_err(data_failure)?;
        // Preserve the original missing-project precedence over the related read.
        let session = if manuscript.rows.is_empty() {
            None
        } else {
            Some(sessions.read(&db, &id).map_err(data_failure)?)
        };
        Ok(Input::GetSession {
            manuscript_id: id,
            manuscript,
            session,
        })
    })
    .await
    .map_err(|_| failure("storage_error", "写作会话读取任务未完成。", &key))??;
    let DecidedProject {
        output,
        instance_id,
        request_id,
    } = service.project_decision(key.clone(), input).await?;
    drop(output.input);
    match output.decision {
        Decision::Session { session }
            if session
                .as_ref()
                .is_none_or(|session| session.manuscript_id == manuscript_id.trim()) =>
        {
            Ok(session)
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
            "写作会话结果种类不匹配。",
            &key,
            &instance_id,
            &request_id,
        )),
    }
}

#[tauri::command]
pub(crate) async fn save_runtime_writing_session(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    draft: WritingSessionDraft,
) -> NativeResult<()> {
    save_session(
        store.shared_agent_connection(),
        Arc::clone(service.inner()),
        Arc::clone(catalog.inner()),
        package_key,
        draft,
    )
    .await
}

pub(super) async fn save_session(
    database: SharedDatabase,
    service: Arc<NativePreviewService>,
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
    draft: WritingSessionDraft,
) -> NativeResult<()> {
    let selected = key.clone();
    // The original Core owner survives a dropped IPC waiter and drains at exit.
    tokio::spawn(async move {
        let _operation = service.project_operation(&selected).await?;
        let (_, resources) = resources(catalog, selected.clone()).await?;
        let probe_database = Arc::clone(&database);
        let probe_key = selected.clone();
        tokio::task::spawn_blocking(move || -> NativeResult<()> {
            // The original entry acquires its database owner before validation.
            // Retain that failure order, while releasing it before Native waits.
            drop(
                probe_database
                    .lock()
                    .map_err(|error| failure("storage_error", error.to_string(), &probe_key))?,
            );
            Ok(())
        })
        .await
        .map_err(|_| failure("storage_error", "写作会话存储核对任务未完成。", &selected))??;
        let DecidedProject {
            output,
            instance_id,
            request_id,
        } = service
            .project_decision(selected.clone(), Input::PrepareSession { draft })
            .await?;
        let Input::PrepareSession { draft } = output.input else {
            return Err(decided_failure(
                "plugin_host_response_mismatch",
                "写作会话输入种类不匹配。",
                &selected,
                &instance_id,
                &request_id,
            ));
        };
        match output.decision {
            Decision::SessionPrepared { draft: prepared } if prepared == draft => {}
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
                    "写作会话准备结果未保持原输入。",
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
            let db = database
                .lock()
                .map_err(|error| failure("storage_error", error.to_string(), &write_key))?;
            // As before, the FK at this autocommit write decides missing M.
            // No additional project lookup or database guard crosses Native.
            resources.save(&db, &draft).map_err(|mut error| {
                error.instance_id = Some(instance_id);
                error.request_id = Some(request_id);
                error
            })
        })
        .await
        .map_err(|_| {
            decided_failure(
                "storage_error",
                "写作会话保存结果未知；请读取原会话核对。",
                &selected,
                &saved_instance,
                &saved_request,
            )
        })?
    })
    .await
    .map_err(|_| {
        failure(
            "storage_error",
            "写作会话保存任务未能结算；请读取原会话核对。",
            &key,
        )
    })?
}
