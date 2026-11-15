//! Original history and environment endpoints on the selected writing package.
use super::{domain::Domain, repository, resources::Sql, tools};
use crate::{
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{NativePreviewService, NativeResult},
        writing_projects::{context::Binding, failure},
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_compile::{
    CompileRun, Decision, Input, LatexToolchainInfo,
};
use std::sync::Arc;

pub(super) async fn task<T: Send + 'static>(
    store: AppStore,
    binding: Binding,
    action: impl FnOnce(AppStore, Domain, Sql) -> NativeResult<T> + Send + 'static,
) -> NativeResult<T> {
    task_with_operation(store, binding, move |store, domain, sql, _operation| {
        action(store, domain, sql)
    })
    .await
}

pub(super) async fn task_with_operation<T: Send + 'static>(
    store: AppStore,
    binding: Binding,
    action: impl FnOnce(
        AppStore,
        Domain,
        Sql,
        crate::runtime_plugins::native::ProjectOperation,
    ) -> NativeResult<T>
    + Send
    + 'static,
) -> NativeResult<T> {
    let key = binding.package_key.clone();
    let selected = key.clone();
    tokio::spawn(async move {
        let service = binding.service.clone();
        let operation = service.project_operation(&selected).await?;
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let domain = Domain::new(binding, runtime);
            let sql = domain.resources()?;
            action(store, domain, sql, operation)
        })
        .await
        .map_err(|_| failure("storage_error", "写作编译读取任务未完成。", &selected))?
    })
    .await
    .map_err(|_| failure("plugin_host_call_failed", "写作编译读取等待未完成。", &key))?
}

enum ReadRun {
    Run(String),
    Latest(String),
    LatestPdf(String),
}

async fn read(
    store: AppStore,
    binding: Binding,
    request: ReadRun,
) -> NativeResult<Option<CompileRun>> {
    task(store, binding, |store, domain, sql| {
        let run = {
            let db = store
                .connection()
                .map_err(|cause| domain.error(cause.into()))?;
            match request {
                ReadRun::Run(id) => repository::get_run(&sql, &db, id.trim()).map(Some),
                ReadRun::Latest(id) => repository::latest_run(&sql, &db, id.trim()),
                ReadRun::LatestPdf(id) => repository::latest_pdf_run(&sql, &db, id.trim()),
            }
            .map_err(|cause| domain.error(cause))?
        };
        run.map(|run| domain.run(run)).transpose()
    })
    .await
}

pub(super) fn detect(store: &AppStore, domain: &Domain) -> NativeResult<LatexToolchainInfo> {
    let checked_at: String = {
        let db = store
            .connection()
            .map_err(|cause| domain.error(cause.into()))?;
        db.query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })
        .map_err(|cause| domain.error(cause.into()))?
    };
    let tools = tools::inspect(&store.writing_cache_directory().join("toolchain-home"));
    match domain.call(Input::ProjectToolchain {
        tools: tools.clone(),
        checked_at: checked_at.clone(),
    })? {
        Decision::Toolchain { toolchain }
            if toolchain.tools == tools && toolchain.checked_at == checked_at =>
        {
            Ok(toolchain)
        }
        _ => Err(domain.mismatch()),
    }
}

#[tauri::command]
pub(crate) async fn detect_runtime_latex_toolchain(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
) -> NativeResult<LatexToolchainInfo> {
    let binding = Binding {
        service: Arc::clone(service.inner()),
        catalog: Arc::clone(catalog.inner()),
        package_key,
    };
    task(store.inner().clone(), binding, |store, domain, _| {
        detect(&store, &domain)
    })
    .await
}

#[tauri::command]
pub(crate) async fn get_runtime_manuscript_compile_run(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    run_id: String,
) -> NativeResult<CompileRun> {
    let binding = Binding {
        service: Arc::clone(service.inner()),
        catalog: Arc::clone(catalog.inner()),
        package_key: package_key.clone(),
    };
    read(store.inner().clone(), binding, ReadRun::Run(run_id))
        .await?
        .ok_or_else(|| {
            failure(
                "plugin_host_response_mismatch",
                "写作编译读取缺少运行记录。",
                &package_key,
            )
        })
}

#[tauri::command]
pub(crate) async fn get_latest_runtime_manuscript_compile_run(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Option<CompileRun>> {
    read(
        store.inner().clone(),
        Binding {
            service: Arc::clone(service.inner()),
            catalog: Arc::clone(catalog.inner()),
            package_key,
        },
        ReadRun::Latest(manuscript_id),
    )
    .await
}

#[tauri::command]
pub(crate) async fn get_latest_pdf_runtime_manuscript_compile_run(
    store: tauri::State<'_, AppStore>,
    service: tauri::State<'_, Arc<NativePreviewService>>,
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
    package_key: String,
    manuscript_id: String,
) -> NativeResult<Option<CompileRun>> {
    read(
        store.inner().clone(),
        Binding {
            service: Arc::clone(service.inner()),
            catalog: Arc::clone(catalog.inner()),
            package_key,
        },
        ReadRun::LatestPdf(manuscript_id),
    )
    .await
}
