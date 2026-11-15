//! Borrowed writing transactions use the same catalog projection and host owner.
use super::*;
use crate::runtime_plugins::literature_bibliography::Resources;
use research_workbench_native_sdk::catalog::{CatalogInput, CatalogOutput, CatalogRequest};

pub(crate) struct DecidedBibliography {
    pub(crate) output: CatalogOutput,
    pub(crate) instance_id: String,
    pub(crate) request_id: String,
}
impl LiteratureTraceService {
    pub(crate) async fn bibliography_operation(
        &self,
        key: &str,
    ) -> NativeResult<tokio::sync::RwLockReadGuard<'_, ()>> {
        let guard = self.doi_operations.read().await;
        self.available().map_err(|error| error.package(key))?;
        Ok(guard)
    }
    pub(crate) async fn bibliography_resources(&self, key: &str) -> NativeResult<Resources> {
        self.available().map_err(|error| error.package(key))?;
        let catalog = Arc::clone(&self.catalog);
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_catalog(&catalog, &key)?;
            Resources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&key))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_accept_failed",
                "文献书目来源资源核对任务未结算。",
            )
        })?
    }
    /// Caller retains bibliography_operation across the whole writing owner. This
    /// inner exchange must not acquire a second read gate behind shutdown's writer.
    pub(crate) async fn bibliography_call(
        &self,
        key: &str,
        input: CatalogInput,
    ) -> NativeResult<DecidedBibliography> {
        let accepted = match &input {
            CatalogInput::BibliographySearch { .. } => true,
            CatalogInput::PlanBibliographyCandidates { .. } => true,
            CatalogInput::Project { input } => {
                matches!(input.request, CatalogRequest::Get { .. }) && input.items.len() == 1
            }
            _ => false,
        };
        if !accepted {
            return Err(Self::error(
                "plugin_host_profile_unsupported",
                "书目来源只接纳候选计划与已有题录投影。",
            ));
        }
        #[cfg(not(unix))]
        {
            let _ = input;
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            )
            .package(key))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            self.available().map_err(|error| error.package(key))?;
            let result = async {
                let host = self.ensure_host(&mut slot, key).await?;
                let output = host.catalog_call(input, Duration::from_secs(60)).await?;
                let (instance_id, request_id) = host.request_identity().ok_or_else(|| {
                    host.error(
                        "plugin_host_response_mismatch",
                        "文献书目结果缺少实际请求身份。",
                    )
                })?;
                Ok(DecidedBibliography {
                    output,
                    instance_id,
                    request_id,
                })
            }
            .await;
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|error| error.package(key))
        }
    }
}
