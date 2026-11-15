//! One immutable preparation, followed by the original Core-owned transaction.
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::literature_duplicate_merge::{self, MergeOutcome, MergePhase, MergeResources},
};
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    duplicate_review::MergeOperation,
};

mod errors;

struct MergeCall {
    candidate_id: String,
    survivor_item_id: String,
}

impl LiteratureTraceService {
    pub(crate) async fn merge_duplicate_candidate(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        candidate_id: String,
        survivor_item_id: String,
    ) -> NativeResult<MergeOperation> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let result = tokio::spawn(async move {
            owner
                .merge_duplicate_owned(database, &selected, candidate_id, survivor_item_id)
                .await
        })
        .await
        .map_err(|_| errors::unknown("文献合并任务未能结算，禁止自动重试。"))
        .and_then(|result| result);
        result.map_err(|mut error| {
            error.package_key = Some(key);
            if error.duplicate_merge.is_none() {
                errors::set_not_started(&mut error);
            }
            error
        })
    }

    async fn duplicate_merge_resources(&self, key: &str) -> NativeResult<MergeResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_duplicate_merge(&catalog, &selected)?;
            MergeResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message))
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "文献合并资源核对任务未完成。"))?
    }

    async fn merge_duplicate_owned(
        &self,
        database: SharedDatabase,
        key: &str,
        candidate_id: String,
        survivor_item_id: String,
    ) -> NativeResult<MergeOperation> {
        let resources = Arc::new(self.duplicate_merge_resources(key).await?);
        #[cfg(not(unix))]
        {
            let _ = (database, resources, candidate_id, survivor_item_id);
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
            let mut before = slot.as_ref().and_then(NativeHost::request_identity);
            let result = self
                .run_duplicate_merge(
                    &mut slot,
                    database,
                    key,
                    resources,
                    MergeCall {
                        candidate_id,
                        survivor_item_id,
                    },
                    &mut before,
                )
                .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|error| errors::scope_request(error, before.as_ref()))
        }
    }

    #[cfg(unix)]
    async fn run_duplicate_merge(
        &self,
        slot: &mut Option<NativeHost>,
        database: SharedDatabase,
        key: &str,
        resources: Arc<MergeResources>,
        call: MergeCall,
        before: &mut Option<(String, String)>,
    ) -> NativeResult<MergeOperation> {
        if resources.package_key() != key {
            return Err(Self::error(
                "plugin_host_response_mismatch",
                "文献合并资源不属于本次插件包。",
            ));
        }
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        let capture_db = Arc::clone(&database);
        let capture_resources = Arc::clone(&resources);
        // Pending-candidate query and its original row conversions precede
        // both BEGIN and host startup/preparation waits.
        let captured = tokio::task::spawn_blocking(move || {
            let guard = capture_db
                .lock()
                .map_err(|cause| errors::owner(MergePhase::Header, cause))?;
            literature_duplicate_merge::capture(
                &guard,
                &capture_resources,
                call.candidate_id,
                call.survivor_item_id,
            )
            .map_err(|problem| errors::problem(problem, MergeOutcome::NotStarted))
        })
        .await
        .map_err(|_| errors::capture_failed())?
        .map_err(|error| match slot.as_ref() {
            Some(host) => errors::bind(host, error),
            None => error,
        })?;
        self.available()?;
        let host = self.ensure_host(slot, key).await?;
        *before = host.request_identity();
        let output = tokio::select! {
            biased;
            _=&mut stopped=>Err(host.error("plugin_host_stopping","应用正在关闭文献合并准备。")),
            output=host.catalog_call(CatalogInput::PrepareDuplicateMerge {
                input: Box::new(captured.input.clone()),
            },Duration::from_secs(60))=>output,
        }?;
        let CatalogOutput::DuplicateMergePreparation { outcome: plan } = output else {
            return Err(host.error("plugin_host_response_mismatch", "文献合并准备阶段不匹配。"));
        };
        literature_duplicate_merge::verify_preparation(&captured, &plan).map_err(|problem| {
            errors::bind(host, errors::problem(problem, MergeOutcome::NotStarted))
        })?;
        self.available()
            .map_err(|error| errors::bind(host, error))?;
        // No stop/select surrounds this worker. The owned task and the same
        // host mutex retain it until the original transaction is settled.
        tokio::task::spawn_blocking(move || {
            let mut guard = database
                .lock()
                .map_err(|cause| errors::owner(MergePhase::Begin, cause))?;
            literature_duplicate_merge::apply(&mut guard, &resources, &captured, &plan)
                .map_err(errors::failure)
        })
        .await
        .map_err(|_| {
            errors::bind(
                host,
                errors::unknown("文献合并数据库任务未能结算，禁止自动重试。"),
            )
        })?
        .map_err(|error| errors::bind(host, error))
    }
}
