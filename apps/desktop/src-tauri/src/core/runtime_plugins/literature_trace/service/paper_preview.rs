//! Ordered paper snapshot consumers on the existing managed host and Core owner.
use super::*;
use crate::{
    agent::{reading_context::ReadingNeed, reading_sources, reading_submission},
    ai::paper_reading_contracts::{PaperContextError, PaperContextPreview, PaperContextRequest},
    data_interface::{PaperPrepareIntent, PaperPrepareStatus, SqlImmediateReceipt},
    hbs_storage::boundary::{BoundaryError, HbsOutcome},
    runtime_plugins::literature_context::preview::PreviewResources,
    storage::AppStore,
};
mod failure;
mod reader;
mod retrieval;
use failure::Error;
use reader::Reader;
use serde_json::Value;

#[derive(Clone)]
enum Operation {
    Preview(PaperContextRequest),
    TurnSources(reading_sources::Request),
    Status(Arc<PaperPrepareIntent>),
    Replay(Arc<PaperPrepareIntent>),
    Prepare {
        intent: Arc<PaperPrepareIntent>,
        model: Arc<(
            research_agent_core::protocol::ModelIdentity,
            Option<String>,
            Option<String>,
        )>,
    },
}
impl Operation {
    fn is_paper(&self) -> bool {
        matches!(
            self,
            Self::Status(_) | Self::Replay(_) | Self::Prepare { .. }
        )
    }
    fn is_prepare(&self) -> bool {
        matches!(self, Self::Prepare { .. })
    }
}
enum Output {
    Preview(PaperContextPreview),
    TurnSources(reading_sources::Response),
    Status(PaperPrepareStatus),
    Replay(
        Option<
            Box<(
                research_agent_core::submission::SubmissionRecord,
                SqlImmediateReceipt,
            )>,
        >,
    ),
    Prepared(
        Box<(
            research_agent_core::submission::SubmissionRecord,
            SqlImmediateReceipt,
        )>,
    ),
}
enum Step {
    Complete(Output),
    Need(ReadingNeed),
}
impl LiteratureTraceService {
    pub(crate) async fn preview_paper_context(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: PaperContextRequest,
    ) -> Result<PaperContextPreview, Value> {
        request
            .validate()
            .map_err(|error| Error::domain(error).into_wire())?;
        let selected = key.clone();
        match self
            .paper_snapshot_operation(store, key, Operation::Preview(request))
            .await
            .map_err(|error| error.into_wire_with_package(&selected))?
        {
            Output::Preview(preview) => Ok(preview),
            _ => Err(Error::mismatch().into_wire()),
        }
    }
    // The caller retains the already accepted original service operation.
    pub(super) async fn preview_analysis_context_in_operation(
        &self,
        store: AppStore,
        key: String,
        request: PaperContextRequest,
    ) -> Result<PaperContextPreview, crate::ai::error::AiError> {
        request
            .validate()
            .map_err(|error| Error::domain(error).into_analysis_ai(&key))?;
        let selected = key.clone();
        match self
            .paper_snapshot_in_operation(store, key, Operation::Preview(request))
            .await
            .map_err(|error| error.into_analysis_ai(&selected))?
        {
            Output::Preview(preview) => Ok(preview),
            _ => Err(Error::mismatch().into_analysis_ai(&selected)),
        }
    }
    pub(crate) async fn read_paper_turn_sources(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        request: reading_sources::Request,
    ) -> Result<reading_sources::Response, Value> {
        reading_sources::validate_request(&request).map_err(|error| serde_json::json!(error))?;
        let selected = key.clone();
        match self
            .paper_snapshot_operation(store, key, Operation::TurnSources(request))
            .await
            .map_err(|error| error.into_wire_with_package(&selected))?
        {
            Output::TurnSources(sources) => Ok(sources),
            _ => Err(Error::mismatch().into_wire()),
        }
    }
    pub(crate) async fn paper_prepare_status(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        intent: PaperPrepareIntent,
    ) -> Result<PaperPrepareStatus, reading_submission::Error> {
        let intent = Arc::new(intent);
        let selected = key.clone();
        match self
            .paper_snapshot_operation(store, key, Operation::Status(Arc::clone(&intent)))
            .await
            .map_err(|error| error.into_submission_error(&selected, &intent.request))?
        {
            Output::Status(status) => Ok(status),
            _ => Err(Error::mismatch().into_submission_error(&selected, &intent.request)),
        }
    }
    pub(crate) async fn replay_paper_submission(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        intent: PaperPrepareIntent,
    ) -> Result<
        Option<(
            research_agent_core::submission::SubmissionRecord,
            SqlImmediateReceipt,
        )>,
        reading_submission::Error,
    > {
        let intent = Arc::new(intent);
        let selected = key.clone();
        match self
            .paper_snapshot_operation(store, key, Operation::Replay(Arc::clone(&intent)))
            .await
            .map_err(|error| error.into_submission_error(&selected, &intent.request))?
        {
            Output::Replay(record) => Ok(record.map(|record| *record)),
            _ => Err(Error::mismatch().into_submission_error(&selected, &intent.request)),
        }
    }
    pub(crate) async fn prepare_paper_submission(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        intent: PaperPrepareIntent,
        model: (
            research_agent_core::protocol::ModelIdentity,
            Option<String>,
            Option<String>,
        ),
    ) -> Result<
        (
            research_agent_core::submission::SubmissionRecord,
            SqlImmediateReceipt,
        ),
        reading_submission::Error,
    > {
        let intent = Arc::new(intent);
        let selected = key.clone();
        let operation = Operation::Prepare {
            intent: Arc::clone(&intent),
            model: Arc::new(model),
        };
        match self
            .paper_snapshot_operation(store, key, operation)
            .await
            .map_err(|error| error.into_submission_error(&selected, &intent.request))?
        {
            Output::Prepared(record) => Ok(*record),
            _ => Err(Error::mismatch().into_submission_error(&selected, &intent.request)),
        }
    }
    async fn paper_snapshot_operation(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        operation: Operation,
    ) -> Result<Output, Error> {
        let owner = Arc::clone(self);
        let paper = operation.is_paper();
        tokio::spawn(async move { owner.paper_snapshot_owned(store, key, operation).await })
            .await
            .map_err(|_| {
                if paper {
                    Error::paper_unsettled()
                } else {
                    Error::unsettled()
                }
            })
            .and_then(|result| result)
    }
    async fn paper_snapshot_owned(
        &self,
        store: AppStore,
        key: String,
        operation: Operation,
    ) -> Result<Output, Error> {
        let paper = operation.is_paper();
        if !paper {
            self.available().map_err(Error::Native)?;
        }
        let _lifetime = self.doi_operations.read().await;
        self.paper_snapshot_in_operation(store, key, operation)
            .await
    }

    async fn paper_snapshot_in_operation(
        &self,
        store: AppStore,
        key: String,
        operation: Operation,
    ) -> Result<Output, Error> {
        let paper = operation.is_paper();
        if !paper {
            self.available().map_err(Error::Native)?;
        }
        // Preserve the original Core binding and submitted-source checks before
        // accepting a package or starting a Native exchange.
        let source_capture = match &operation {
            Operation::Preview(_)
            | Operation::Status(_)
            | Operation::Replay(_)
            | Operation::Prepare { .. } => None,
            Operation::TurnSources(request) => {
                let store = store.clone();
                let request = request.clone();
                let captured = tokio::task::spawn_blocking(move || {
                    let db = store.connection().map_err(|_| {
                        crate::ai::structured_resource_validation::Failure::original(
                            reading_sources::Error::Unavailable,
                        )
                    })?;
                    reading_sources::capture_sources(&db, &request)
                })
                .await
                .map_err(|_| Error::sources(reading_sources::Error::Unavailable))?
                .map_err(Error::source_failure)?;
                Some(captured)
            }
        };
        let resources = if paper {
            None
        } else {
            Some(
                self.paper_preview_resources(&key)
                    .await
                    .map_err(Error::Native)?,
            )
        };
        #[cfg(not(unix))]
        {
            let _ = (resources, store, operation, source_capture);
            Err(Error::Native(Self::error(
                "plugin_host_platform_unsupported",
                "此平台尚未提供文献宿主。",
            )))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            if !paper {
                self.available().map_err(Error::Native)?;
            }
            let mut outcome = HbsOutcome::NotStarted;
            let result = async {
                if !paper {
                    self.ensure_host(&mut slot, &key).await?;
                }
                let mut reader = resources.map(Reader::new).unwrap_or_else(Reader::pending);
                reader.source_capture = source_capture;
                loop {
                    let (next, result) =
                        match reader::attempt(store.clone(), operation.clone(), reader).await {
                            Ok(result) => result,
                            Err(error) => return Ok(Err(error)),
                        };
                    reader = next;
                    let step = match result {
                        Ok(step) => step,
                        Err(error) => return Ok(Err(error)),
                    };
                    match step {
                        Step::Complete(output) => return Ok(Ok(output)),
                        Step::Need(ReadingNeed::Resources) => {
                            if operation.is_prepare() {
                                outcome = HbsOutcome::RolledBack;
                            }
                            self.available()?;
                            reader.resources = Some(self.paper_preview_resources(&key).await?);
                        }
                        Step::Need(ReadingNeed::TraceResources) => {
                            outcome = HbsOutcome::RolledBack;
                            self.available()?;
                            reader.trace_resources = Some(self.paper_trace_resources(&key).await?);
                        }
                        Step::Need(ReadingNeed::Trace(prepared)) => {
                            outcome = HbsOutcome::RolledBack;
                            let host = self.ensure_host(&mut slot, &key).await?;
                            let (prepared, decision, origin) =
                                self.paper_trace_decision(host, prepared).await?;
                            reader.trace = Some(reader::ResolvedTrace {
                                prepared,
                                decision,
                                origin,
                            });
                        }
                        Step::Need(ReadingNeed::Preview(capture)) => {
                            if operation.is_prepare() {
                                outcome = HbsOutcome::RolledBack;
                            }
                            let host = self.ensure_host(&mut slot, &key).await?;
                            let decision = self
                                .preview_context_stage(host, capture.input.clone())
                                .await?;
                            let Some((instance_id, native_request_id)) = host.request_identity()
                            else {
                                return Err(Self::error(
                                    "plugin_host_response_mismatch",
                                    "论文预览缺少本次纯决定身份。",
                                ));
                            };
                            reader.push(
                                *capture,
                                decision,
                                crate::ai::structured_resource_validation::DecisionOrigin {
                                    package_key: key.clone(),
                                    instance_id,
                                    native_request_id,
                                },
                            );
                        }
                    }
                }
            }
            .await;
            let before_exchange = result
                .as_ref()
                .is_err_and(|error| error.request_id.is_none());
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|mut error| {
                    if before_exchange {
                        error.request_id = None;
                    }
                    error
                })
                .map_err(|error| {
                    if paper {
                        Error::PaperHost { error, outcome }
                    } else {
                        Error::Native(error)
                    }
                })?
        }
    }
    async fn paper_preview_resources(&self, key: &str) -> NativeResult<PreviewResources> {
        let catalog = Arc::clone(&self.catalog);
        let selected = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_paper_context_preview(&catalog, &selected)?;
            PreviewResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_resource_worker_unsettled",
                "论文快照资源检查未确认完成。",
            )
        })?
    }
}
