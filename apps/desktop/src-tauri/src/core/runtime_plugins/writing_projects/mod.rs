//! Fixed Core persistence adaptation. Project decisions and DTOs come from Native.
pub(crate) mod citation_metadata;
pub(crate) mod compile;
pub(crate) mod context;
pub(crate) mod creation;
pub(crate) mod files;
pub(crate) mod host_resources;
pub(crate) mod metadata;
pub(crate) mod recovery;
mod resources;
pub(crate) mod sessions;
pub(crate) mod status;
use crate::{
    data_interface::{SharedDatabase, plugin_resources::PluginDataError},
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{
            DecidedProject, NativeHostError, NativePreviewService, NativeProfile, NativeResult,
            package,
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_project::{Decision, Input, Manuscript};
use resources::ProjectReadResources;
use std::sync::Arc;

fn failure(code: &str, message: impl Into<String>, key: &str) -> Box<NativeHostError> {
    NativeHostError::for_profile(NativeProfile::WritingProjects, code, message).package(key)
}
fn data_failure(cause: Box<PluginDataError>) -> Box<NativeHostError> {
    let code = if cause.original_sql_failure.is_some() {
        "storage_error"
    } else {
        cause.code
    };
    let mut error = failure(
        code,
        format!("{}：{}", cause.message, cause.safe_cause),
        &cause.package_key,
    );
    error.data_error = Some(cause);
    error
}
fn decided_failure(
    code: &str,
    message: impl Into<String>,
    key: &str,
    instance_id: &str,
    request_id: &str,
) -> Box<NativeHostError> {
    let mut error = failure(code, message, key);
    error.instance_id = Some(instance_id.into());
    error.request_id = Some(request_id.into());
    error
}
async fn load(
    catalog: Arc<RuntimePluginCatalog>,
    key: String,
) -> NativeResult<ProjectReadResources> {
    let selected = key.clone();
    tokio::task::spawn_blocking(move || {
        let package = package::accept(&catalog, &selected, NativeProfile::WritingProjects)?;
        ProjectReadResources::load(&catalog, &package)
            .map_err(|error| failure(error.code, error.message, &selected))
    })
    .await
    .map_err(|_| {
        failure(
            "plugin_host_accept_failed",
            "写作资源核对任务未完成。",
            &key,
        )
    })?
}

async fn capture(
    database: SharedDatabase,
    resources: ProjectReadResources,
    key: String,
    id: Option<String>,
) -> NativeResult<Input> {
    let selected = key.clone();
    tokio::task::spawn_blocking(move || {
        let db = database
            .lock()
            .map_err(|cause| failure("storage_error", cause.to_string(), &selected))?;
        match id {
            None => Ok(Input::ListProjects {
                rows: resources.list(&db).map_err(data_failure)?,
            }),
            Some(id) => Ok(Input::GetProject {
                rows: resources.get(&db, &id).map_err(data_failure)?,
                manuscript_id: id,
            }),
        }
    })
    .await
    .map_err(|_| failure("storage_error", "写作工程读取任务未完成。", &key))?
}

#[tauri::command]
pub(crate) async fn list_runtime_manuscripts(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
) -> NativeResult<Vec<Manuscript>> {
    list_projects(
        store.shared_agent_connection(),
        service.inner(),
        Arc::clone(catalog.inner()),
        package_key,
    )
    .await
}

async fn list_projects(
    database: SharedDatabase,
    service: &NativePreviewService,
    catalog: Arc<RuntimePluginCatalog>,
    package_key: String,
) -> NativeResult<Vec<Manuscript>> {
    let resources = load(catalog, package_key.clone()).await?;
    let input = capture(database, resources, package_key.clone(), None).await?;
    let decided = service.project_decision(package_key.clone(), input).await?;
    let DecidedProject {
        output,
        instance_id,
        request_id,
    } = decided;
    drop(output.input);
    match output.decision {
        Decision::Projects { items } => Ok(items),
        Decision::Rejected { reason } => Err(decided_failure(
            reason.code(),
            reason.to_string(),
            &package_key,
            &instance_id,
            &request_id,
        )),
        _ => Err(decided_failure(
            "plugin_host_response_mismatch",
            "写作列表结果种类不匹配。",
            &package_key,
            &instance_id,
            &request_id,
        )),
    }
}

#[tauri::command]
pub(crate) async fn get_runtime_manuscript(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Manuscript> {
    get_project(
        store.shared_agent_connection(),
        service.inner(),
        Arc::clone(catalog.inner()),
        package_key,
        manuscript_id,
    )
    .await
}

async fn get_project(
    database: SharedDatabase,
    service: &NativePreviewService,
    catalog: Arc<RuntimePluginCatalog>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Manuscript> {
    let resources = load(catalog, package_key.clone()).await?;
    let input = capture(
        database,
        resources,
        package_key.clone(),
        Some(manuscript_id.trim().into()),
    )
    .await?;
    let decided = service.project_decision(package_key.clone(), input).await?;
    project_reply(&package_key, decided)
}

fn project_reply(package_key: &str, decided: DecidedProject) -> NativeResult<Manuscript> {
    let DecidedProject {
        output,
        instance_id,
        request_id,
    } = decided;
    let manuscript_id = match output.input {
        Input::GetProject { manuscript_id, .. } | Input::FinishStatus { manuscript_id, .. } => {
            manuscript_id
        }
        _ => {
            return Err(decided_failure(
                "plugin_host_response_mismatch",
                "写作工程投影未返回原读取或状态执行输入。",
                package_key,
                &instance_id,
                &request_id,
            ));
        }
    };
    match output.decision {
        Decision::Project { manuscript } if manuscript.id == manuscript_id => Ok(manuscript),
        Decision::Rejected { reason } => Err(decided_failure(
            reason.code(),
            reason.to_string(),
            package_key,
            &instance_id,
            &request_id,
        )),
        _ => Err(decided_failure(
            "plugin_host_response_mismatch",
            "写作工程结果种类不匹配。",
            package_key,
            &instance_id,
            &request_id,
        )),
    }
}

#[cfg(all(test, unix))]
mod tests;

fn sql_error(key: &str, phase: &'static str, cause: &rusqlite::Error) -> Box<NativeHostError> {
    let diagnostic = crate::runtime_plugins::native::SafeDatabaseError::sql(phase, cause);
    let mut error = failure("storage_error", diagnostic.to_string(), key);
    error.save_database_error = Some(Box::new(diagnostic));
    error
}
