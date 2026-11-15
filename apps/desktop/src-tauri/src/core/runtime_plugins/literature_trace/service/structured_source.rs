//! One Core-owned structured-source read, including snapshot/proof publication.
use super::*;
use crate::{
    agent::context_session::{self, resource_tools::structured_runtime},
    ai::structured_resource_validation::Failure,
    hbs_storage::boundary::{BoundaryFailure, HbsOutcome},
    runtime_plugins::{
        literature_context::PaperContextResources, literature_hbs::HbsOperationError,
    },
    storage::AppStore,
};
use research_agent_core::{model::ToolCall, runner::ToolOutput, submission::SubmissionRecord};
use serde_json::{Value, json};
#[cfg(unix)]
mod calls;
#[cfg(all(test, unix))]
mod tests;
mod worker;

impl LiteratureTraceService {
    /// Core-only fixed readers shared by PDF views and the original text writer.
    pub(crate) async fn paper_context_read_resources(
        &self,
        key: String,
    ) -> NativeResult<PaperContextResources> {
        self.available()?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        tokio::task::spawn_blocking(move || {
            let package = package::accept_paper_context(&catalog, &selected)?;
            package::accept_reading_evidence(&catalog, &selected)?;
            PaperContextResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| {
            Self::error(
                "plugin_host_source_worker_unsettled",
                "论文上下文读取准备未确认完成。",
            )
            .package(&key)
        })?
    }
}

pub(crate) enum SourceReadError {
    Context(Failure<context_session::Error>),
    Data(Box<BoundaryFailure<crate::ai::error::AiError>>),
    Native(Box<HbsOperationError>),
    Writing(Box<crate::agent::writing_submission::WritingSubmissionError>),
    Published {
        snapshot_id: String,
        proof_confirmed: bool,
        phase: &'static str,
        outcome: HbsOutcome,
        failure: Box<Failure<context_session::Error>>,
    },
}
impl std::fmt::Debug for SourceReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Context(_) => formatter.write_str("SourceReadError::Context"),
            Self::Data(error) => formatter
                .debug_struct("SourceReadError::Data")
                .field("phase", &error.phase)
                .field("outcome", &error.outcome)
                .finish(),
            Self::Native(_) => formatter.write_str("SourceReadError::Native"),
            Self::Writing(_) => formatter.write_str("SourceReadError::Writing"),
            Self::Published {
                proof_confirmed,
                outcome,
                ..
            } => formatter
                .debug_struct("SourceReadError::Published")
                .field("proof_confirmed", proof_confirmed)
                .field("outcome", outcome)
                .finish(),
        }
    }
}
impl SourceReadError {
    pub(crate) fn missing_binding() -> Self {
        Self::native(
            LiteratureTraceService::error(
                "plugin_host_package_unavailable",
                "当前会话未绑定可用的文献来源插件。",
            ),
            "admission",
            HbsOutcome::NotStarted,
        )
    }
    pub(super) fn context(error: context_session::Error) -> Self {
        Self::Context(Failure::original(error))
    }
    fn native(error: Box<NativeHostError>, phase: &'static str, outcome: HbsOutcome) -> Self {
        Self::Native(Box::new(HbsOperationError::native(error, phase, outcome)))
    }
    fn unsettled() -> Self {
        Self::native(
            LiteratureTraceService::error(
                "structured_source_worker_unsettled",
                "论文来源读取未确认结算；请核对原调用与快照，不要自动重复写入。",
            ),
            "execution",
            HbsOutcome::Unknown,
        )
    }
    pub(crate) fn into_output(self, record: &SubmissionRecord, call: &ToolCall) -> ToolOutput {
        let mut durable_snapshot = None;
        let mut proof_confirmed = None;
        let (message, diagnostic) = match self {
            Self::Writing(error) => return error.into_source_tool_output(record, call),
            Self::Native(error) => error.into_wire(),
            Self::Data(error) => HbsOperationError::Boundary(BoundaryFailure {
                failure: error.failure.map_error(|error| error.to_string()),
                phase: error.phase,
                outcome: error.outcome,
                rollback_error: error.rollback_error,
            })
            .into_wire(),
            Self::Context(failure) => HbsOperationError::Boundary(BoundaryFailure {
                failure: failure.map_error(|error| error.to_string()),
                phase: "source-read",
                outcome: HbsOutcome::NotStarted,
                rollback_error: None,
            })
            .into_wire(),
            Self::Published {
                snapshot_id,
                proof_confirmed: confirmed,
                phase,
                outcome,
                failure,
            } => {
                durable_snapshot = Some(snapshot_id);
                proof_confirmed = Some(confirmed);
                HbsOperationError::Boundary(BoundaryFailure {
                    failure: failure.map_error(|error| error.to_string()),
                    phase,
                    outcome,
                    rollback_error: None,
                })
                .into_wire()
            }
        };
        let mut value = json!({"error":message,"requestId":record.request_id,"turnId":record.turn_id,"callId":call.call_id,
            "guidance":"Re-list resources or re-read current authorized content; never use the original PDF as a fallback. A snapshot may already be durable; do not treat a failed read as a new authorization or blindly repeat writes."});
        if let Some(diagnostic) = diagnostic {
            value["nativeFailure"] = json!(diagnostic);
        }
        if let Some(snapshot) = durable_snapshot {
            value["durableSnapshotId"] = json!(snapshot);
        }
        if let Some(confirmed) = proof_confirmed {
            value["readProofConfirmed"] = json!(confirmed);
        }
        ToolOutput {
            content: value.to_string(),
            is_error: true,
        }
    }
}

impl LiteratureTraceService {
    pub(crate) async fn read_structured_source(
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
                .read_structured_source_owned(store, key, record, call)
                .await
        })
        .await
        .map_err(|_| SourceReadError::unsettled())
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
    async fn read_structured_source_owned(
        &self,
        store: AppStore,
        key: String,
        record: SubmissionRecord,
        call: ToolCall,
    ) -> Result<Value, SourceReadError> {
        // Preserve original live authorization before package/host admission.
        worker::authorize(store.clone(), record.clone(), call.clone()).await?;
        self.available()
            .map_err(|error| SourceReadError::native(error, "admission", HbsOutcome::NotStarted))?;
        let _lifetime = self.doi_operations.read().await;
        self.available()
            .map_err(|error| SourceReadError::native(error, "admission", HbsOutcome::NotStarted))?;
        let catalog = Arc::clone(&self.catalog);
        let selected = key.clone();
        let resources = tokio::task::spawn_blocking(move || {
            let package = package::accept_paper_context(&catalog, &selected)?;
            package::accept_reading_evidence(&catalog, &selected)?;
            PaperContextResources::load(&catalog, &package)
                .map_err(|error| Self::error(error.code, error.message).package(&selected))
        })
        .await
        .map_err(|_| SourceReadError::unsettled())?
        .map_err(|error| SourceReadError::native(error, "admission", HbsOutcome::NotStarted))?;
        #[cfg(not(unix))]
        {
            let _ = (resources, store, record, call);
            Err(SourceReadError::native(
                Self::error(
                    "plugin_host_platform_unsupported",
                    "此平台尚未提供文献宿主。",
                ),
                "admission",
                HbsOutcome::NotStarted,
            ))
        }
        #[cfg(unix)]
        {
            let mut slot = self.host.lock().await;
            self.available().map_err(|error| {
                SourceReadError::native(error, "admission", HbsOutcome::NotStarted)
            })?;
            let mut before = None;
            let result = async {
                let host = self.ensure_host(&mut slot, &key).await?;
                before = host.request_identity();
                self.run_structured_source(host, store, &key, record, call, resources)
                    .await
            }
            .await;
            let no_stage_request = result
                .as_ref()
                .is_err_and(|error| error.request_id.is_none());
            self.finish_domain(&mut slot, result)
                .await
                .map_err(|mut error| {
                    // Cleanup may bind the host's last successful request again.
                    // Keep a pre-exchange error free of that unrelated ID.
                    if no_stage_request
                        || before.as_ref().is_some_and(|(instance, request)| {
                            error.instance_id.as_ref() == Some(instance)
                                && error.request_id.as_ref() == Some(request)
                        })
                    {
                        error.request_id = None;
                    }
                    SourceReadError::native(error, "source-preparation", HbsOutcome::NotStarted)
                })?
        }
    }
    #[cfg(unix)]
    async fn run_structured_source(
        &self,
        host: &mut NativeHost,
        store: AppStore,
        key: &str,
        record: SubmissionRecord,
        call: ToolCall,
        resources: PaperContextResources,
    ) -> NativeResult<Result<Value, SourceReadError>> {
        if resources.package_key() != key {
            return Err(Self::error(
                "plugin_host_response_mismatch",
                "论文上下文纯结果与本次阶段或输入不匹配。",
            ));
        }
        let mut cache = worker::Cache::default();
        loop {
            self.available()?;
            let (next, step) = match worker::attempt(
                store.clone(),
                record.clone(),
                call.clone(),
                resources.clone(),
                cache,
            )
            .await
            {
                Ok(result) => result,
                Err(error) => return Ok(Err(error)),
            };
            cache = next;
            match step {
                Err(error) => return Ok(Err(error)),
                Ok(worker::Step::Complete(value)) => return Ok(Ok(value)),
                Ok(worker::Step::Build(capture)) => {
                    let capture = Arc::new(capture);
                    let prepared = self.build_context(host, Arc::clone(&capture)).await?;
                    cache.prepared = Some((capture, prepared));
                }
                Ok(worker::Step::Project(input)) => {
                    let input = Arc::new(input);
                    match self.project_context(host, key, Arc::clone(&input)).await? {
                        Ok(projection) => cache.projected = Some((input, projection)),
                        Err(failure) => {
                            return Ok(Err(SourceReadError::Data(Box::new(BoundaryFailure {
                                failure,
                                phase: "source-validation",
                                outcome: HbsOutcome::NotStarted,
                                rollback_error: None,
                            }))));
                        }
                    }
                }
                Ok(worker::Step::Digest { capture, input }) => {
                    let digest = self.digest_context(host, input).await?;
                    cache.digested = Some((*capture, digest));
                }
            }
        }
    }
}
