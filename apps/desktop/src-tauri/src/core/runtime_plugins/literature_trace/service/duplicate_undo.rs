//! Only immutable undo preparation crosses the process boundary.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::literature_duplicate_undo::{self, UndoOutcome, UndoResources},
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    duplicate_undo::UndoMergePrepareInputV1,
};

mod errors;

impl LiteratureTraceService {
    pub(crate) async fn undo_merge_operation(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        operation_id: String,
    ) -> NativeResult<()> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let diagnostic_operation_id = operation_id.clone();
        let result = tokio::spawn(async move {
            owner
                .undo_merge_owned(database, &selected, operation_id)
                .await
        })
        .await
        .map_err(|_| errors::unknown("合并撤销任务未能结算，禁止自动重试。"))
        .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            error.operation_id = Some(diagnostic_operation_id);
            if error.duplicate_undo.is_none() {
                errors::set_not_started(&mut error);
            }
            error
        })
    }

    async fn duplicate_undo_resources(&self, key: &str) -> NativeResult<UndoResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_duplicate_undo(&catalog, &selected)?;
            UndoResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "合并撤销资源核对任务未完成。"))?
    }

    async fn undo_merge_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        operation_id: String,
    ) -> NativeResult<()> {
        let resources = self.duplicate_undo_resources(key).await?;
        #[cfg(not(unix))]
        {
            let _ = (database, resources, operation_id);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            let mut slot = self
                .host
                .try_lock()
                .map_err(|_| Self::error("plugin_host_busy", "文献原生实例已有在途请求。"))?;
            self.available()?;
            let mut before = None;
            let result = async {
                if resources.package_key() != key {
                    return Err(Self::error(
                        "plugin_host_response_mismatch",
                        "合并撤销资源不属于本次插件包。",
                    ));
                }
                let host = self.ensure_host(&mut slot, key).await?;
                before = host.request_identity();
                self.run_duplicate_undo(host, database, resources, operation_id)
                    .await
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|error| errors::scope_request(error, before.as_ref()))
        }
    }

    #[cfg(unix)]
    async fn run_duplicate_undo(
        &self,
        host: &mut NativeHost,
        database: SharedDatabase,
        resources: UndoResources,
        operation_id: String,
    ) -> NativeResult<()> {
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let capture_db = Arc::clone(&database);
        let capture_resources = resources.clone();
        let captured = tokio::task::spawn_blocking(move || {
            let guard = capture_db.lock().map_err(errors::owner)?;
            literature_duplicate_undo::capture(&guard, &capture_resources, operation_id)
                .map_err(|problem| errors::problem(problem, UndoOutcome::NotStarted))
        })
        .await
        .map_err(|_| host.error("storage_error", "合并撤销读取任务未完成。"))?
        .map_err(|error| errors::bind(host, error))?;
        self.available()
            .map_err(|error| errors::bind(host, error))?;
        let input = UndoMergePrepareInputV1 {
            snapshot_json: captured.snapshot_json.clone(),
        };
        let output = tokio::select! {
            biased;
            _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭合并撤销准备。")),
            output=host.catalog_call(CatalogInput::PrepareDuplicateUndo {input},Duration::from_secs(60))=>output,
        }?;
        let CatalogOutput::DuplicateUndoPreparation { outcome: plan } = output else {
            return Err(host.error("plugin_host_response_mismatch", "合并撤销准备阶段不匹配。"));
        };
        self.available()
            .map_err(|error| errors::bind(host, error))?;
        // Once this worker is dispatched, the owned task and host mutex wait
        // for its original transaction; dropping a client wait cannot abort it.
        tokio::task::spawn_blocking(move || {
            let mut guard = database.lock().map_err(errors::owner)?;
            literature_duplicate_undo::apply(&mut guard, &resources, &captured, &plan)
                .map_err(errors::failure)
        })
        .await
        .map_err(|_| {
            errors::bind(
                host,
                errors::unknown("合并撤销数据库任务未能结算，禁止自动重试。"),
            )
        })?
        .map_err(|error| errors::bind(host, error))
    }
}
