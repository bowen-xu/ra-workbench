//! Original Core source/legacy operations, with only two selected snapshot reads.
use super::*;
use crate::{
    agent::writing_resources::WritingResourceSource,
    data_interface::{SharedDatabase, guarded_transaction},
    runtime_plugins::reading_evidence::ReadingEvidenceResources,
    structured_resources::{
        self, LegacyImportDraft, Resource, StartupImportDraft, StartupImportResult,
    },
};
use serde_json::Value;

enum Operation {
    Startup {
        draft: StartupImportDraft,
        writing: Option<WritingResourceSource>,
    },
    Legacy(LegacyImportDraft),
    Sources(String),
}
enum Output {
    Startup(StartupImportResult),
    Resources(Vec<Resource>),
    Sources(Value),
}

impl LiteratureTraceService {
    pub(crate) async fn structured_startup(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: Option<String>,
        draft: StartupImportDraft,
        writing: Option<WritingResourceSource>,
    ) -> NativeResult<StartupImportResult> {
        match self
            .reading_operation(database, key, Operation::Startup { draft, writing })
            .await?
        {
            Output::Startup(value) => Ok(value),
            _ => Err(Self::error(
                "plugin_host_response_mismatch",
                "启动导入结果种类不匹配。",
            )),
        }
    }
    pub(crate) async fn structured_legacy(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: Option<String>,
        draft: LegacyImportDraft,
    ) -> NativeResult<Vec<Resource>> {
        match self
            .reading_operation(database, key, Operation::Legacy(draft))
            .await?
        {
            Output::Resources(value) => Ok(value),
            _ => Err(Self::error(
                "plugin_host_response_mismatch",
                "旧资料导入结果种类不匹配。",
            )),
        }
    }
    pub(crate) async fn structured_sources(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: Option<String>,
        id: String,
    ) -> NativeResult<Value> {
        match self
            .reading_operation(database, key, Operation::Sources(id))
            .await?
        {
            Output::Sources(value) => Ok(value),
            _ => Err(Self::error(
                "plugin_host_response_mismatch",
                "资源来源结果种类不匹配。",
            )),
        }
    }
    async fn reading_operation(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: Option<String>,
        operation: Operation,
    ) -> NativeResult<Output> {
        let owner = Arc::clone(self);
        tokio::spawn(async move { owner.reading_owned(database, key, operation).await })
            .await
            .map_err(|_| {
                Self::error(
                    "structured_resource_worker_unsettled",
                    "结构化资源操作未能确认结算，禁止自动重放。",
                )
            })?
    }
    async fn reading_owned(
        &self,
        database: SharedDatabase,
        key: Option<String>,
        operation: Operation,
    ) -> NativeResult<Output> {
        self.available()?;
        // A missing-provider startup still writes Core templates/contexts. Keep
        // that owned worker under the same existing shutdown lifetime gate.
        let _lifetime = self.doi_operations.read().await;
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let resources = tokio::task::spawn_blocking(move || select(&catalog, key.as_deref()))
            .await
            .map_err(|_| {
                Self::error("plugin_host_accept_failed", "冻结来源资源核对任务未完成。")
            })??;
        self.available()?;
        let Some(resources) = resources else {
            if !matches!(&operation, Operation::Startup { .. }) {
                return Err(unavailable());
            }
            return worker(database, None, operation).await;
        };
        #[cfg(not(unix))]
        {
            let _ = (database, resources, operation);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            let selected = resources.package_key().to_owned();
            let mut slot = self.host.lock().await;
            self.available().map_err(|e| e.package(&selected))?;
            let mut host_ready_for_core = false;
            let result = async {
                let host = self.ensure_host(&mut slot, &selected).await?;
                host_ready_for_core = true;
                let instance = host.request_identity().map(|(instance, _)| instance);
                self.available().map_err(|e| e.package(&selected))?;
                // No business Catalog request was sent. The initialize request
                // must not be attached to a later source SQL or migration error.
                worker(database, Some(resources), operation)
                    .await
                    .map_err(|mut e| {
                        e.package_key = Some(selected.clone());
                        e.instance_id = instance;
                        e.request_id = None;
                        e
                    })
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|mut e| {
                    // Failed-host cleanup may bind its last initialize frame.
                    // That frame is not the identity of this Core SQL worker.
                    if host_ready_for_core {
                        e.request_id = None;
                    }
                    e.package(&selected)
                })
        }
    }
}

fn select(
    catalog: &RuntimePluginCatalog,
    explicit: Option<&str>,
) -> NativeResult<Option<ReadingEvidenceResources>> {
    let key = if let Some(key) = explicit {
        key.to_owned()
    } else {
        let listing = catalog
            .discover()
            .map_err(|e| LiteratureTraceService::error(e.code, e.message))?;
        let providers = listing
            .packages
            .iter()
            .filter(|p| p.manifest["id"] == "literature_library")
            .collect::<Vec<_>>();
        match providers.as_slice() {
            [] => return Ok(None),
            [package] => package.package_key.clone(),
            _ => {
                return Err(LiteratureTraceService::error(
                    "plugin_host_package_ambiguous",
                    "受控目录存在多个文献插件包，冻结来源未选择提供者。",
                ));
            }
        }
    };
    let package = package::accept_reading_evidence(catalog, &key)?;
    ReadingEvidenceResources::load(catalog, &package)
        .map(Some)
        .map_err(|e| LiteratureTraceService::error(e.code, e.message).package(&key))
}
fn unavailable() -> Box<NativeHostError> {
    let error = crate::ai::error::AiCommandError::from(
        crate::ai::error::AiError::CapabilityUnavailable("literature_library.paper_reading_source"),
    );
    LiteratureTraceService::error(error.code(), error.message())
}
fn core_error(error: structured_resources::Error) -> Box<NativeHostError> {
    let mut result = LiteratureTraceService::error(error.command_code(), error.to_string());
    if let structured_resources::Error::Evidence(cause) = error {
        result.data_error = cause.data;
    }
    result
}
async fn worker(
    database: SharedDatabase,
    resources: Option<ReadingEvidenceResources>,
    operation: Operation,
) -> NativeResult<Output> {
    tokio::task::spawn_blocking(move || {
        let mut db = database
            .lock()
            .map_err(|_| LiteratureTraceService::error("storage_error", "结构化资源存储不可用"))?;
        guarded_transaction::run_mut(&mut db, |db| {
            let was_autocommit = db.is_autocommit();
            let source = resources.as_ref().map(|r| {
                r as &dyn crate::ai::structured_resource_validation::ExplicitReadingEvidence
            });
            let result = match operation {
                Operation::Startup { draft, writing } => {
                    structured_resources::import_startup_with_evidence(source, db, draft, writing)
                        .map(Output::Startup)
                }
                Operation::Legacy(draft) => source
                    .ok_or_else(|| structured_resources::Error::Invalid("来源提供者不可用".into()))
                    .and_then(|r| structured_resources::import_legacy_with_evidence(r, db, draft))
                    .map(Output::Resources),
                Operation::Sources(id) => source
                    .ok_or_else(|| structured_resources::Error::Invalid("来源提供者不可用".into()))
                    .and_then(|r| structured_resources::sources_with_evidence(r, db, &id))
                    .map(Output::Sources),
            }
            .map_err(core_error);
            let unsettled = was_autocommit && !db.is_autocommit();
            (result, unsettled)
        })
    })
    .await
    .map_err(|_| {
        LiteratureTraceService::error(
            "structured_resource_worker_unsettled",
            "结构化资源操作未能确认结算，禁止自动重放。",
        )
    })?
}

#[cfg(test)]
mod tests;
