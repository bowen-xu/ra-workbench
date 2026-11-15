//! One owned ordinary-tool invocation; only its metadata projection waits on Native.
use super::*;
use crate::{
    agent::context_session::resource_tools::metadata_runtime::{self, Cache, Step},
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::{
        context_metadata::ContextMetadataResources, literature_hbs::HbsOperationError,
    },
    storage::AppStore,
};
use research_agent_core::{model::ToolCall, submission::SubmissionRecord};
use serde_json::Value;
#[cfg(unix)]
mod calls;
#[cfg(all(test, unix))]
mod tests;

impl LiteratureTraceService {
    pub(crate) async fn read_context_metadata(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        record: SubmissionRecord,
        call: ToolCall,
    ) -> Result<Value, SourceReadError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        tokio::spawn(async move {
            owner
                .read_context_metadata_owned(store, key, record, call)
                .await
        })
        .await
        .map_err(|_| metadata_runtime::worker_unsettled())
        .and_then(|result| result)
        .map_err(|mut failure| {
            if let SourceReadError::Native(native) = &mut failure
                && let HbsOperationError::Native { error, .. } = native.as_mut()
            {
                error.package_key = Some(selected);
            }
            failure
        })
    }

    async fn read_context_metadata_owned(
        &self,
        store: AppStore,
        key: String,
        record: SubmissionRecord,
        call: ToolCall,
    ) -> Result<Value, SourceReadError> {
        let _lifetime = self.doi_operations.read().await;
        let mut resources: Option<ContextMetadataResources> = None;
        let mut cache = Cache::default();
        loop {
            let (next, step) = metadata_runtime::attempt(
                store.clone(),
                record.clone(),
                call.clone(),
                resources.clone(),
                cache,
            )
            .await?;
            cache = next;
            match step? {
                Step::Complete(value) => return Ok(value),
                Step::Writing => continue,
                Step::Resources => {
                    self.available()
                        .map_err(|error| metadata_native_error(error, "admission"))?;
                    let catalog = Arc::clone(&self.catalog);
                    let selected = key.clone();
                    let loaded = tokio::task::spawn_blocking(move || {
                        let package = package::accept_context_metadata(&catalog, &selected)?;
                        ContextMetadataResources::load(&catalog, &package)
                            .map_err(|error| metadata_resource_error(error, &selected))
                    })
                    .await
                    .map_err(|_| metadata_runtime::worker_unsettled())?
                    .map_err(|error| metadata_native_error(error, "admission"))?;
                    if loaded.package_key() != key {
                        return Err(metadata_native_error(
                            Self::error(
                                "plugin_host_response_mismatch",
                                "文献元数据资源不属于本次选定包。",
                            ),
                            "admission",
                        ));
                    }
                    resources = Some(loaded);
                }
                Step::Project(capture) => {
                    #[cfg(not(unix))]
                    {
                        let _ = capture;
                        return Err(metadata_native_error(
                            Self::error(
                                "plugin_host_platform_unsupported",
                                "此平台尚未提供文献宿主。",
                            ),
                            "admission",
                        ));
                    }
                    #[cfg(unix)]
                    {
                        let mut slot = self.host.lock().await;
                        self.available()
                            .map_err(|error| metadata_native_error(error, "admission"))?;
                        let mut before = None;
                        let input = Arc::clone(&capture);
                        let result = async {
                            let host = self.ensure_host(&mut slot, &key).await?;
                            before = host.request_identity();
                            self.project_context_metadata(host, input).await
                        }
                        .await;
                        let no_stage_request = result
                            .as_ref()
                            .is_err_and(|error| error.request_id.is_none());
                        let body =
                            self.finish_domain(&mut slot, result)
                                .await
                                .map_err(|mut error| {
                                    if no_stage_request
                                        || before.as_ref().is_some_and(|(instance, request)| {
                                            error.instance_id.as_ref() == Some(instance)
                                                && error.request_id.as_ref() == Some(request)
                                        })
                                    {
                                        error.request_id = None;
                                    }
                                    metadata_native_error(error, "source-preparation")
                                })?;
                        cache.resolved(capture, body);
                    }
                }
            }
        }
    }
}

fn metadata_native_error(error: Box<NativeHostError>, phase: &'static str) -> SourceReadError {
    SourceReadError::Native(Box::new(HbsOperationError::native(
        error,
        phase,
        HbsOutcome::NotStarted,
    )))
}

/// Only these three accepted resources have a known logical identity here.
/// Manifest/package failures retain their original diagnostics without inventing one.
fn metadata_resource_error(
    error: crate::runtime_plugins::RuntimePluginError,
    package: &str,
) -> Box<NativeHostError> {
    let resource = match error.resource.as_deref() {
        Some("data/context-metadata-header.v1.sql") => Some("header"),
        Some("data/context-metadata-creators.v1.sql") => Some("creators"),
        Some("data/context-metadata-identifiers.v1.sql") => Some("identifiers"),
        _ => None,
    };
    let mut native = LiteratureTraceService::error(error.code, &error.message).package(package);
    if let Some(resource) = resource {
        native.data_error = Some(Box::new(
            crate::data_interface::plugin_resources::PluginDataError {
                code: "plugin_data_resource_unavailable",
                message: "文献元数据固定读取资源不可用。",
                package_key: package.into(),
                resource_id: format!("literature.context-metadata.{resource}"),
                resource_version: 1,
                phase: "resource",
                safe_cause: error.code.into(),
                sqlite_code: None,
                sqlite_extended_code: None,
                original_sql_failure: None,
            },
        ));
    }
    native
}
