//! One owned DOI workflow over existing host, HTTP and Core storage owners.
use super::import_jobs::owner_error;
use super::*;
use crate::{
    data_interface::SharedDatabase,
    runtime_plugins::{
        literature_catalog::data::CatalogResources,
        literature_doi::{DoiResources, storage},
        literature_import_jobs::{ImportJobsProgress, ImportJobsResources, SqlOutcome},
        metadata_http::MetadataHttp,
    },
};
use reqwest::Url;
use research_workbench_native_sdk::{
    catalog::{CatalogInput, CatalogOutput},
    doi_metadata::{
        DoiHttpBodyV1, DoiHttpErrorV1, DoiPreviewInputV1, DoiPreviewOutcomeV1, DoiTranslationV1,
        MetadataPreviewV1,
    },
};

#[cfg(unix)]
mod completion;

#[derive(Clone)]
struct DoiStorage {
    database: SharedDatabase,
    resources: DoiResources,
    jobs: ImportJobsResources,
    catalog: CatalogResources,
}

struct Lookup {
    normalized: String,
    resolver_accept: String,
    publisher_accept: String,
    resolver_limit: usize,
    publisher_limit: usize,
    resolver_override: Option<Url>,
}

impl LiteratureTraceService {
    pub(crate) async fn preview_doi(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        doi: String,
        item_id: Option<String>,
    ) -> NativeResult<MetadataPreviewV1> {
        self.preview_doi_using(database, key, doi, item_id, None)
            .await
    }

    #[cfg(all(debug_assertions, feature = "development-data"))]
    pub(crate) async fn preview_doi_fixture(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        doi: String,
        item_id: Option<String>,
        resolver: Url,
    ) -> NativeResult<MetadataPreviewV1> {
        let loopback = resolver
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|address| address.is_loopback());
        if resolver.scheme() != "http"
            || !loopback
            || !resolver.username().is_empty()
            || resolver.password().is_some()
        {
            return Err(Self::error(
                "invalid_input",
                "DOI开发夹具只允许无认证的回环HTTP地址。",
            ));
        }
        self.preview_doi_using(database, key, doi, item_id, Some(resolver))
            .await
    }

    async fn preview_doi_using(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        doi: String,
        item_id: Option<String>,
        resolver_override: Option<Url>,
    ) -> NativeResult<MetadataPreviewV1> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        // Dropping the caller's wait does not drop an accepted workflow or its
        // original committed prefix. Shutdown drains this same owner.
        tokio::spawn(async move {
            owner
                .doi_owned(database, selected, doi, item_id, resolver_override)
                .await
        })
        .await
        .map_err(|_| {
            ImportJobsProgress::unknown("doi-owner").bind(Self::error(
                "storage_error",
                "DOI操作结果未知，禁止自动重试。",
            ))
        })?
        .map_err(|mut error| {
            error.package_key = Some(key);
            error
        })
    }

    async fn doi_storage(&self, database: SharedDatabase, key: &str) -> NativeResult<DoiStorage> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_doi(&catalog, &key)?;
            let failure = |cause: crate::runtime_plugins::RuntimePluginError| {
                Self::error(cause.code, cause.message)
            };
            Ok(DoiStorage {
                database,
                resources: DoiResources::load(&catalog, &package).map_err(failure)?,
                jobs: ImportJobsResources::load(&catalog, &package).map_err(failure)?,
                catalog: CatalogResources::load(&catalog, &package).map_err(failure)?,
            })
        })
        .await
        .map_err(|_| Self::error("plugin_host_accept_failed", "DOI固定资源核对任务未完成。"))?
    }

    async fn doi_owned(
        self: &Arc<Self>,
        database: SharedDatabase,
        key: String,
        doi: String,
        item_id: Option<String>,
        resolver_override: Option<Url>,
    ) -> NativeResult<MetadataPreviewV1> {
        self.available()?;
        let _operation = self.doi_operations.read().await;
        self.available()?;
        let storage = self.doi_storage(database, &key).await?;
        #[cfg(not(unix))]
        {
            let _ = (storage, doi, item_id, resolver_override);
            Err(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            ))
        }
        #[cfg(unix)]
        {
            let normalized = crate::doi::clean_doi(&doi).map(|value| value.to_lowercase());
            let (started, instance) = self
                .doi_exchange(
                    &key,
                    None,
                    DoiPreviewInputV1::Start {
                        raw_input: doi.clone(),
                        normalized_doi: normalized.clone(),
                    },
                )
                .await?;
            let DoiPreviewOutcomeV1::Started {
                normalized_doi,
                source_kind,
                finish_states,
                resolver_accept,
                publisher_accept,
                resolver_limit,
                publisher_limit,
            } = started
            else {
                return Err(Self::error(
                    "plugin_host_response_mismatch",
                    "DOI准备阶段回复不匹配。",
                ));
            };
            if normalized.as_deref() != Some(normalized_doi.as_str()) {
                return Err(Self::error(
                    "plugin_host_response_mismatch",
                    "DOI准备身份未保持。",
                ));
            }
            let initial = storage.clone();
            let input_doi = doi;
            let normalized_for_header = normalized_doi.clone();
            let (job_id, result_id) = tokio::task::spawn_blocking(move || {
                let mut db = initial.database.lock().map_err(owner_error)?;
                storage::create_job(
                    &mut db,
                    &initial.jobs,
                    &source_kind,
                    &normalized_for_header,
                    &input_doi,
                )
            })
            .await
            .map_err(|_| {
                ImportJobsProgress::unknown("doi-header").bind(Self::error(
                    "storage_error",
                    "DOI任务头结果未知，禁止自动重试。",
                ))
            })??;
            let lookup = Lookup {
                normalized: normalized_doi.clone(),
                resolver_accept,
                publisher_accept,
                resolver_limit,
                publisher_limit,
                resolver_override,
            };
            let translation = match self.lookup_doi(&key, &instance, lookup).await {
                Ok(value) => value,
                Err(error) => {
                    let failure = storage.clone();
                    let message = error.message.clone();
                    let job = job_id.clone();
                    let result = result_id.clone();
                    // The original failed result and finish are independent
                    // autocommits. Their errors precede the lookup failure.
                    tokio::task::spawn_blocking(move || {
                        let db = failure.database.lock().map_err(owner_error)?;
                        storage::fail_job(
                            &db,
                            &failure.jobs,
                            &failure.resources,
                            &job,
                            &result,
                            &message,
                            &finish_states,
                        )
                    })
                    .await
                    .map_err(|_| {
                        progress("doi-failure", &job_id, &result_id).bind(Self::error(
                            "storage_error",
                            "DOI失败记录结果未知，禁止自动重试。",
                        ))
                    })??;
                    let mut recorded = progress("lookup-failure-recorded", &job_id, &result_id);
                    recorded.completed_result_updates = Some(1);
                    recorded.outcome = SqlOutcome::Returned;
                    return Err(recorded.bind(error));
                }
            };
            self.complete_doi(completion::CompleteDoi {
                storage,
                key,
                instance,
                normalized: normalized_doi,
                item_id,
                translation,
                job_id,
                result_id,
            })
            .await
        }
    }

    #[cfg(unix)]
    async fn lookup_doi(
        &self,
        key: &str,
        instance: &str,
        policy: Lookup,
    ) -> NativeResult<DoiTranslationV1> {
        // These are the original two named capability limits, not a general
        // network interface. A plugin cannot remove the owning Core's ceilings.
        if policy.resolver_limit == 0
            || policy.resolver_limit > 4 * 1024 * 1024
            || policy.publisher_limit == 0
            || policy.publisher_limit > 2 * 1024 * 1024
            || policy.resolver_accept.is_empty()
            || policy.publisher_accept.is_empty()
        {
            return Err(Self::error(
                "plugin_host_response_mismatch",
                "DOI读取政策超出已授权边界。",
            ));
        }
        let resolver = match policy.resolver_override {
            Some(resolver) => Ok(resolver),
            None => Url::parse(&format!("https://doi.org/{}", policy.normalized)),
        };
        let (client, resolver_result) = match &resolver {
            Ok(url) => match MetadataHttp::new() {
                Ok(client) => {
                    let response = self
                        .doi_get(
                            &client,
                            url.clone(),
                            &policy.resolver_accept,
                            policy.resolver_limit,
                        )
                        .await?;
                    (Some(client), response)
                }
                Err(cause) => (None, Err(DoiHttpErrorV1::Client { cause })),
            },
            Err(cause) => (
                None,
                Err(DoiHttpErrorV1::InvalidResolver {
                    cause: cause.to_string(),
                }),
            ),
        };
        let (translated, _) = self
            .doi_exchange(
                key,
                Some(instance),
                DoiPreviewInputV1::Translate {
                    normalized_doi: policy.normalized.clone(),
                    resolver: resolver_result.clone(),
                    publisher: None,
                },
            )
            .await?;
        match translated {
            DoiPreviewOutcomeV1::Translated { translation } => Ok(translation),
            DoiPreviewOutcomeV1::NeedsPublisher {} => {
                let (Some(client), Ok(url)) = (client, resolver) else {
                    return Err(Self::error(
                        "plugin_host_response_mismatch",
                        "DOI出版商读取缺少原请求。",
                    ));
                };
                let publisher = self
                    .doi_get(
                        &client,
                        url,
                        &policy.publisher_accept,
                        policy.publisher_limit,
                    )
                    .await?;
                let (complete, _) = self
                    .doi_exchange(
                        key,
                        Some(instance),
                        DoiPreviewInputV1::Translate {
                            normalized_doi: policy.normalized,
                            resolver: resolver_result,
                            publisher: Some(publisher),
                        },
                    )
                    .await?;
                match complete {
                    DoiPreviewOutcomeV1::Translated { translation } => Ok(translation),
                    _ => Err(Self::error(
                        "plugin_host_response_mismatch",
                        "DOI最终译码回复不匹配。",
                    )),
                }
            }
            _ => Err(Self::error(
                "plugin_host_response_mismatch",
                "DOI译码回复不匹配。",
            )),
        }
    }

    #[cfg(unix)]
    async fn doi_get(
        &self,
        client: &MetadataHttp,
        url: Url,
        accept: &str,
        limit: usize,
    ) -> NativeResult<Result<DoiHttpBodyV1, DoiHttpErrorV1>> {
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        self.available()?;
        tokio::select! {
            biased;
            _ = &mut stopped => Err(Self::error("plugin_host_stopping", "应用正在关闭DOI操作。")),
            result = client.get(url, accept, limit) => Ok(result),
        }
    }

    #[cfg(unix)]
    async fn doi_exchange(
        &self,
        key: &str,
        instance: Option<&str>,
        input: DoiPreviewInputV1,
    ) -> NativeResult<(DoiPreviewOutcomeV1, String)> {
        let mut slot = self.host.lock().await;
        self.available()?;
        let mut before = None;
        let result = async {
            let host = if let Some(instance) = instance { pinned_host(&mut slot, key, instance)? }
                else { self.ensure_host(&mut slot, key).await? };
            before = host.request_identity();
            let stopped = self.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            self.available()?;
            let start = matches!(input, DoiPreviewInputV1::Start { .. });
            let expected_doi = match &input {
                DoiPreviewInputV1::Start { normalized_doi, .. } => normalized_doi.clone(),
                _ => None,
            };
            let publisher = matches!(&input, DoiPreviewInputV1::Translate { publisher: Some(_), .. });
            let output = tokio::select! {
                biased;
                _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭DOI准备。")),
                value = host.catalog_call(CatalogInput::DoiPreview { input: Box::new(input) }, Duration::from_secs(60)) => value,
            }?;
            let CatalogOutput::DoiPreview { outcome } = output else {
                return Err(host.error("plugin_host_response_mismatch", "DOI回复种类不匹配。"));
            };
            let outcome = *outcome;
            if let DoiPreviewOutcomeV1::Rejected { code, message } = outcome {
                if (start && code == "invalid_input") || (!start && code == "connector_error") {
                    return Err(host.error(&code, message));
                }
                return Err(host.error("plugin_host_response_mismatch", "DOI拒绝结果与阶段不匹配。"));
            }
            let valid = if start { matches!(&outcome, DoiPreviewOutcomeV1::Started { .. }) }
                else { matches!(&outcome, DoiPreviewOutcomeV1::Translated { .. }) || (!publisher && matches!(&outcome, DoiPreviewOutcomeV1::NeedsPublisher {})) };
            if !valid { return Err(host.error("plugin_host_response_mismatch", "DOI回复阶段不匹配。")); }
            if let DoiPreviewOutcomeV1::Started {
                normalized_doi, resolver_accept, publisher_accept, resolver_limit, publisher_limit, ..
            } = &outcome
                && (expected_doi.as_deref() != Some(normalized_doi.as_str())
                    || resolver_accept.is_empty() || publisher_accept.is_empty()
                    || *resolver_limit == 0 || *resolver_limit > 4 * 1024 * 1024
                    || *publisher_limit == 0 || *publisher_limit > 2 * 1024 * 1024)
            {
                return Err(host.error("plugin_host_response_mismatch", "DOI准备身份或读取边界不匹配。"));
            }
            let identity = host.request_identity().ok_or_else(|| host.error("plugin_host_response_mismatch", "DOI回复缺少请求身份。"))?.0;
            Ok((outcome, identity))
        }.await;
        self.finish_domain(&mut slot, result)
            .await
            .map_err(|mut error| {
                if let Some((instance, request)) = before.as_ref()
                    && error.instance_id.as_ref() == Some(instance)
                    && error.request_id.as_ref() == Some(request)
                {
                    error.request_id = None;
                }
                error
            })
    }
}

#[cfg(unix)]
fn pinned_host<'a>(
    slot: &'a mut Option<NativeHost>,
    key: &str,
    instance: &str,
) -> NativeResult<&'a mut NativeHost> {
    let host = slot.as_mut().ok_or_else(|| {
        LiteratureTraceService::error(
            "plugin_host_instance_changed",
            "DOI获取期间原生实例已变化，此请求不会自动重放。",
        )
    })?;
    if host.package_key() != key
        || host.is_failed()
        || host
            .request_identity()
            .is_none_or(|(actual, _)| actual != instance)
    {
        let mut error = LiteratureTraceService::error(
            "plugin_host_instance_changed",
            "DOI获取期间原生实例已变化，此请求不会自动重放。",
        );
        error.instance_id = Some(instance.into());
        return Err(error);
    }
    Ok(host)
}

fn progress(phase: &'static str, job_id: &str, result_id: &str) -> ImportJobsProgress {
    let mut value = ImportJobsProgress::new(phase);
    value.job_id = Some(job_id.into());
    value.result_id = Some(result_id.into());
    value.header_committed = Some(true);
    value
}

#[cfg(all(test, unix))]
mod tests;
