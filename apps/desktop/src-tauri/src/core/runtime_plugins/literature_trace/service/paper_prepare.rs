//! Existing reading and snapshot preparation through the selected literature host.
use super::*;
use crate::{
    ai::{
        error::AiError,
        paper_reading_contracts::{
            PaperAiReadingPrepareResult, Preparation, PrepareRequest, PreparedPaperContext,
        },
        structured_resource_validation::DecisionOrigin,
    },
    runtime_plugins::literature_context::preparation::PreparationResources,
    storage::AppStore,
};
use serde_json::Value;
mod failure;
mod worker;
use failure::{Error, Progress};
use worker::{Cache, Need, Step};
mod bootstrap;
#[derive(Clone)]
enum Operation {
    Reading {
        paper: String,
        attachments: Vec<String>,
    },
    PortraitSources {
        paper: String,
        attachments: Vec<String>,
    },
    Bootstrap {
        paper: String,
        attachments: Vec<String>,
    },
    Snapshot(Arc<PrepareRequest>),
    Analysis(Arc<PrepareRequest>),
    Docling(Arc<PrepareRequest>),
}
impl Operation {
    fn scope(&self) -> (&str, &[String]) {
        match self {
            Self::Reading { paper, attachments }
            | Self::PortraitSources { paper, attachments }
            | Self::Bootstrap { paper, attachments } => (paper, attachments),
            Self::Snapshot(request) | Self::Analysis(request) | Self::Docling(request) => {
                (&request.paper_id, &request.attachment_ids)
            }
        }
    }
    fn is_snapshot(&self) -> bool {
        matches!(self, Self::Snapshot(_))
    }
}
enum Output {
    Bootstrap(
        research_workbench_native_sdk::reading_history::contracts::PaperReadingBootstrapOptions,
    ),
    Docling {
        prepared: Box<PaperAiReadingPrepareResult>,
        snapshot: Preparation,
        assets: std::collections::BTreeMap<String, crate::ai::reading_contracts::StoredImageAsset>,
    },
    Reading(Box<PaperAiReadingPrepareResult>),
    Snapshot(Preparation),
    Analysis {
        prepared: Box<PaperAiReadingPrepareResult>,
        snapshot: Preparation,
    },
}
impl LiteratureTraceService {
    pub(crate) async fn prepare_reading_bootstrap(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        paper: String,
        attachments: Vec<String>,
    ) -> Result<Value, Value> {
        match self
            .prepare_paper_operation(
                store,
                key.clone(),
                Operation::Bootstrap { paper, attachments },
            )
            .await?
        {
            Output::Bootstrap(options) => Ok(serde_json::json!(options)),
            _ => Err(Error::mismatch().into_wire(false, &key, None)),
        }
    }
    pub(crate) async fn prepare_portrait_sources(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        paper: String,
        attachments: Vec<String>,
    ) -> Result<PaperAiReadingPrepareResult, AiError> {
        let selected = key.clone();
        let owner = Arc::clone(self);
        let output = tokio::spawn(async move {
            owner
                .prepare_paper_owned(
                    store,
                    key,
                    Operation::PortraitSources { paper, attachments },
                )
                .await
        })
        .await
        .map_err(|_| Error::unsettled().into_ai(&selected, None))?
        .map_err(|(error, progress)| error.into_ai(&selected, progress))?;
        match output {
            Output::Reading(result) => Ok(*result),
            _ => Err(Error::mismatch().into_ai(&selected, None)),
        }
    }

    pub(crate) async fn prepare_docling_context(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: PrepareRequest,
    ) -> Result<
        (
            PaperAiReadingPrepareResult,
            Preparation,
            std::collections::BTreeMap<String, crate::ai::reading_contracts::StoredImageAsset>,
        ),
        AiError,
    > {
        let selected = key.clone();
        let owner = Arc::clone(self);
        let output = tokio::spawn(async move {
            owner
                .prepare_paper_owned(store, key, Operation::Docling(Arc::new(request)))
                .await
        })
        .await
        .map_err(|_| Error::unsettled().into_ai(&selected, None))?
        .map_err(|(error, progress)| error.into_ai(&selected, progress))?;
        match output {
            Output::Docling {
                prepared,
                snapshot,
                assets,
            } => Ok((*prepared, snapshot, assets)),
            _ => Err(Error::mismatch().into_ai(&selected, None)),
        }
    }
    pub(crate) async fn prepare_analysis_context(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: PrepareRequest,
    ) -> Result<(PaperAiReadingPrepareResult, Preparation), Value> {
        match self
            .prepare_paper_operation(store, key.clone(), Operation::Analysis(Arc::new(request)))
            .await?
        {
            Output::Analysis { prepared, snapshot } => Ok((*prepared, snapshot)),
            _ => Err(Error::mismatch().into_wire(false, &key, None)),
        }
    }
    pub(crate) async fn prepare_paper_reading_context(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        paper: String,
        attachments: Vec<String>,
    ) -> Result<PaperAiReadingPrepareResult, Value> {
        match self
            .prepare_paper_operation(
                store,
                key.clone(),
                Operation::Reading { paper, attachments },
            )
            .await?
        {
            Output::Reading(result) => Ok(*result),
            _ => Err(Error::mismatch().into_wire(false, &key, None)),
        }
    }
    pub(crate) async fn prepare_paper_context(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: PrepareRequest,
    ) -> Result<Preparation, Value> {
        request
            .validate()
            .map_err(|error| serde_json::json!(error))?;
        match self
            .prepare_paper_operation(store, key.clone(), Operation::Snapshot(Arc::new(request)))
            .await?
        {
            Output::Snapshot(result) => Ok(result),
            _ => Err(Error::mismatch().into_wire(true, &key, None)),
        }
    }
    async fn prepare_paper_operation(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        operation: Operation,
    ) -> Result<Output, Value> {
        let snapshot = operation.is_snapshot();
        let selected = key.clone();
        let owner = Arc::clone(self);
        tokio::spawn(async move { owner.prepare_paper_owned(store, key, operation).await })
            .await
            .map_err(|_| Error::unsettled().into_wire(snapshot, &selected, None))?
            .map_err(|(error, progress)| error.into_wire(snapshot, &selected, progress))
    }
    async fn prepare_paper_owned(
        &self,
        store: AppStore,
        key: String,
        operation: Operation,
    ) -> Result<Output, (Error, Option<Progress>)> {
        self.available()
            .map_err(|error| (Error::Native(error), None))?;
        let _lifetime = self.doi_operations.read().await;
        self.available()
            .map_err(|error| (Error::Native(error), None))?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let analysis = matches!(
            &operation,
            Operation::Analysis(_) | Operation::Docling(_) | Operation::PortraitSources { .. }
        );
        let bootstrap = matches!(&operation, Operation::Bootstrap { .. });
        let resources = tokio::task::spawn_blocking(move || {
            if analysis {
                package::accept_analysis_workflow(&catalog, &selected)?;
            }
            if bootstrap {
                package::accept_reading_history(&catalog, &selected)?;
            }
            let package = package::accept_paper_context_prepare(&catalog, &selected)?;
            let resources = if analysis || bootstrap {
                PreparationResources::load_context(&catalog, &package)
            } else {
                PreparationResources::load(&catalog, &package)
            };
            let resources = resources
                .map_err(|error| Self::error(error.code, error.message).package(&selected))?;
            let history = if bootstrap {
                Some(
                    crate::runtime_plugins::literature_reading_history::resources::ReadingHistoryResources::load(&catalog, &package)
                        .map_err(|error| Self::error(error.code, error.message).package(&selected))?
                )
            } else {
                None
            };
            Ok::<_, Box<NativeHostError>>((resources, history))
        })
        .await
        .map_err(|_| (Error::unsettled(), None))?
        .map_err(|error| (Error::Native(error), None))?;
        #[cfg(not(unix))]
        {
            let _ = (resources, store, operation);
            Err((
                Error::Native(Self::error(
                    "plugin_host_platform_unsupported",
                    "此平台尚未提供文献宿主。",
                )),
                None,
            ))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            self.available()
                .map_err(|error| (Error::Native(error), None))?;
            let (resources, history) = resources;
            let mut cache = Cache::default();
            cache.bootstrap =
                history.map(|resources| Box::new(bootstrap::BootstrapCache::new(resources)));
            let resources = Arc::new(resources);
            let mut progress = None;
            let result = async {
                let host = self.ensure_host(&mut slot, &key).await?;
                loop {
                    let (next, result) = match worker::attempt(
                        store.clone(), operation.clone(), Arc::clone(&resources), cache,
                    ).await {
                        Ok(value) => value,
                        Err(error) => return Ok(Err((error, None))),
                    };
                    cache = next;
                    progress = Some(cache.progress());
                    let step = match result {
                        Ok(value) => value,
                        Err(error) => return Ok(Err((error, Some(cache.progress())))),
                    };
                    match step {
                        Step::Complete(result) => return Ok(Ok(result)),
                        Step::Need(need) => match *need {
                            Need::ReadingHistory(input) => {
                                let decided = self.reading_history_stage(host, &key, input).await?;
                                let Some(bootstrap) = cache.bootstrap.as_mut() else {
                                    return Ok(Err((Error::mismatch(), progress)));
                                };
                                if let Err(error) = bootstrap.push(decided) {
                                    return Ok(Err((error, progress)));
                                }
                            }
                            Need::AnalysisHistory(input) => {
                                let encoded = research_workbench_native_sdk::paper_context_wire::encode_payload(&input)
                                    .map_err(|_| Self::error(
                                        "plugin_host_source_input_invalid", "旧阅读结构化历史无法编码。",
                                    ))?;
                                let ((result, input), origin) = self.analysis_history_stage(
                                    host, &key, (encoded.clone(), encoded),
                                ).await?;
                                let observed = match Self::observed_analysis_history(result, input, origin) {
                                    Ok(value) => value,
                                    Err(error) => {
                                        use crate::runtime_plugins::literature_analysis::failure::Error as HistoryError;
                                        let error = match error {
                                            HistoryError::Original(error) => Error::original(error, "source-read"),
                                            HistoryError::Native(error) => Error::Native(error),
                                            HistoryError::Rejected { message, origin } => Error::rejected(message, origin),
                                        };
                                        return Ok(Err((error, progress)));
                                    }
                                };
                                let Some(bootstrap) = cache.bootstrap.as_mut() else {
                                    return Ok(Err((Error::mismatch(), progress)));
                                };
                                bootstrap.history = Some(observed);
                            }
                            Need::Build(input) => {
                                let prepared = self.build_context(host, Arc::clone(&input)).await?;
                                cache.built = Some((input, Arc::new(prepared)));
                            }
                            Need::Policy(capture) => {
                                let decision = self.prepare_context_stage(host, capture.input.clone()).await?;
                                let Some((instance_id, native_request_id)) = host.request_identity() else {
                                    return Err(Self::error(
                                        "plugin_host_response_mismatch", "阅读准备缺少本次决定身份。",
                                    ));
                                };
                                cache.push(*capture, decision, DecisionOrigin {
                                    package_key: key.clone(), instance_id, native_request_id,
                                });
                            }
                            Need::Project(input) => {
                                match self.project_context(host, &key, Arc::clone(&input)).await? {
                                    Ok(projection) => cache.projected = Some((input, projection)),
                                    Err(failure) => {
                                        let boundary = crate::hbs_storage::boundary::BoundaryFailure {
                                            failure,
                                            phase: "validation",
                                            outcome: crate::hbs_storage::boundary::HbsOutcome::NotStarted,
                                            rollback_error: None,
                                        };
                                        return Ok(Err((Error::Boundary(Box::new(boundary)), Some(cache.progress()))));
                                    }
                                }
                            }
                            Need::Digest { capture, input } => {
                                let digest = self.digest_context(host, input).await?;
                                cache.digested = Some((*capture, digest));
                            }
                        }
                    }
                }
            }.await;
            let before_exchange = result
                .as_ref()
                .is_err_and(|error| error.request_id.is_none());
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|mut error| {
                    if before_exchange {
                        error.request_id = None;
                    }
                    (Error::Native(error), progress)
                })?
        }
    }
}
