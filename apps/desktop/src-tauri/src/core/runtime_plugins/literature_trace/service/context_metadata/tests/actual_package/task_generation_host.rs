//! Test adapter for original Core stages/Agent I/O and actual Native domain calls.
use super::*;
use crate::{
    agent::{
        model::{
            AgentHost, AgentModelResolver, PreparedAgentModel,
            responses_transport::PreviewResponsesWire,
        },
        service::AgentSessionService,
    },
    ai::{
        CancellationSignal, JsonCredentialStore,
        docling_execution::DoclingExecution,
        docling_generation_support::{GenerationAccess, StageCompletion},
        error::AiError,
        generation_support::{
            AiRun, GenerationMessage, GenerationResponse, OverviewHost, ProviderUsage, hash_text,
            provider_messages_input, validate_evidence_bundle,
        },
        reading_contracts::LlmModelSelection,
    },
    storage::AppStore,
};
use research_agent_core::{
    model::{ModelCapabilities, ModelFuture},
    protocol::ModelIdentity,
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering},
    },
    time::Instant,
};
use uuid::Uuid;

pub(super) struct Wire {
    pub payload: Mutex<Value>,
    pub status: AtomicU16,
    pub calls: AtomicUsize,
    pub pause: Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
}
impl PreviewResponsesWire for Wire {
    fn respond(&self, request: reqwest::Request) -> ModelFuture<'_, reqwest::Response> {
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        let payload = self.payload.lock().unwrap().clone();
        assert!(body.to_string().contains(payload["kind"].as_str().unwrap()));
        assert!(body.to_string().contains("Research question."));
        assert!(body.to_string().contains("Do not invoke any tool."));
        assert!(!body["tools"].as_array().unwrap().is_empty());
        self.calls.fetch_add(1, Ordering::SeqCst);
        let status = self.status.load(Ordering::SeqCst);
        let pause = self.pause.lock().unwrap().take();
        Box::pin(async move {
            if let Some((started, release)) = pause {
                started.notify_one();
                release.notified().await;
            }
            if status != 200 {
                return Ok(hyper::Response::builder()
                    .status(status)
                    .body(reqwest::Body::from("synthetic private provider failure"))
                    .unwrap()
                    .into());
            }
            let item = json!({"id":"suggestion-final","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":payload.to_string()}]});
            let body: String = [
                json!({"type":"response.created","response":{"id":"suggestions","model":"gpt-5.6-terra"}}),
                json!({"type":"response.output_item.done","output_index":0,"item":item}),
                json!({"type":"response.completed","response":{"id":"suggestions","model":"gpt-5.6-terra","status":"completed","output":[item],"usage":{"input_tokens":11,"output_tokens":7}}}),
            ].into_iter().map(|event| format!("data: {event}\n\n")).collect();
            Ok(hyper::Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .body(reqwest::Body::from(body))
                .unwrap()
                .into())
        })
    }
}

pub(super) struct Host {
    pub run: AiRun,
    pub page_commit_unknown: AtomicBool,
    store: AppStore,
    execution: DoclingExecution,
    signal: CancellationSignal,
}
impl Host {
    pub fn new(
        f: &Fixture,
        _credentials: JsonCredentialStore,
        run: AiRun,
        signal: CancellationSignal,
        payload: Value,
    ) -> (Self, Arc<Wire>) {
        let wire = Arc::new(Wire {
            payload: Mutex::new(payload),
            status: AtomicU16::new(200),
            calls: AtomicUsize::new(0),
            pause: Mutex::new(None),
        });
        let transport = wire.clone();
        let models = AgentModelResolver::for_preview(
            &f.store,
            Arc::new(move |host, selection| {
                assert!(matches!(host, AgentHost::Review));
                let selection = selection.unwrap();
                assert_eq!(
                    (&*selection.provider_id, &*selection.model_id),
                    ("openai", "gpt-5.6-terra")
                );
                PreparedAgentModel::for_preview(
                    ModelIdentity {
                        provider_id: selection.provider_id,
                        model_id: selection.model_id,
                        revision: None,
                    },
                    None,
                    None,
                    ModelCapabilities {
                        vision: false,
                        tools: false,
                        parallel_tools: false,
                        reasoning_efforts: vec![],
                    },
                    transport.clone(),
                    Uuid::from_u128(0x517),
                )
            }),
        )
        .unwrap();
        let sessions = Arc::new(AgentSessionService::from_app_store(&f.store).unwrap());
        let execution = DoclingExecution::new(f.store.clone(), models, Some(sessions));
        (
            Self {
                run,
                page_commit_unknown: AtomicBool::new(false),
                store: f.store.clone(),
                execution,
                signal,
            },
            wire,
        )
    }
    async fn data<T: Send + 'static>(
        &self,
        call: impl FnOnce(&mut GenerationAccess<'_, '_>) -> Result<T, AiError> + Send + 'static,
    ) -> Result<T, AiError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut db = store.connection()?;
            call(&mut GenerationAccess::new(&mut db))
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
    }
    pub async fn terminal_failure(&self, error: &AiError) -> Result<(), AiError> {
        let run = self.run.id.clone();
        let status = if error.code() == "canceled" {
            "canceled"
        } else {
            "failed"
        };
        let code = error.code();
        let message = error.to_string();
        self.data(move |data| data.finish(&run, status, Some(code), Some(&message), None))
            .await
    }
}
impl OverviewHost for Host {
    type Model = LlmModelSelection;
    async fn resolve_stage(&self) -> Result<String, AiError> {
        let started = Instant::now();
        let stage = self
            .begin("resolving_scope", Some(self.run.scope_hash.clone()))
            .await?;
        if self.signal.race(async {}).await.is_err() {
            self.fail(&stage, &AiError::Canceled, started).await?;
            return Err(AiError::Canceled);
        }
        let hash = hash_text(&format!("{}:resolving_scope", self.run.scope_hash));
        self.complete(&stage, Some(hash.clone()), started).await?;
        Ok(hash)
    }
    async fn begin(&self, kind: &str, input: Option<String>) -> Result<String, AiError> {
        let run = self.run.id.clone();
        let kind = kind.to_owned();
        self.data(move |data| data.begin(&run, &kind, input.as_deref()))
            .await
    }
    async fn complete(
        &self,
        stage: &str,
        output: Option<String>,
        started: Instant,
    ) -> Result<(), AiError> {
        let stage = stage.to_owned();
        self.data(move |data| {
            data.complete(
                &stage,
                StageCompletion {
                    output_hash: output.as_deref(),
                    input_tokens: None,
                    output_tokens: None,
                    duration_ms: i64::try_from(started.elapsed().as_millis()).unwrap(),
                },
            )
        })
        .await
    }
    async fn fail(&self, stage: &str, error: &AiError, started: Instant) -> Result<(), AiError> {
        crate::ai::generation_support::fail_task_stage(&self.store, stage, error, started).await
    }
    fn progress(&self) {}
    async fn validate_evidence(
        &self,
        run: &AiRun,
        bundle: &crate::ai::candidate_contracts::AiEvidenceBundle,
    ) -> Result<(), AiError> {
        if matches!(
            run.task_kind.as_str(),
            "reference_extraction" | "matrix_synthesis"
        ) {
            let binding = self.store.native_note_trace().unwrap();
            return binding
                .service
                .validate_research_evidence(&binding.package_key, run, bundle)
                .await;
        }
        validate_evidence_bundle(bundle, None)
    }
    async fn messages(
        &self,
        run: &AiRun,
        bundle: &crate::ai::candidate_contracts::AiEvidenceBundle,
    ) -> Result<Vec<GenerationMessage>, AiError> {
        let binding = self.store.native_note_trace().unwrap();
        if matches!(
            run.task_kind.as_str(),
            "reference_extraction" | "matrix_synthesis"
        ) {
            return binding
                .service
                .research_messages(&binding.package_key, run, bundle)
                .await;
        }
        binding
            .service
            .suggestion_messages(&binding.package_key, run, bundle)
            .await
    }
    async fn resolve_bundle(
        &self,
        run: &AiRun,
    ) -> Result<crate::ai::candidate_contracts::AiEvidenceBundle, AiError> {
        if self.page_commit_unknown.swap(false, Ordering::SeqCst) {
            self.store.connection()?.commit_hook(Some(|| true));
        }
        let binding = self.store.native_note_trace().unwrap();
        if run.task_kind == "matrix_synthesis" {
            return binding
                .service
                .persist_matrix_task_evidence(
                    self.store.clone(),
                    binding.package_key.clone(),
                    run.id.clone(),
                )
                .await;
        }
        if run.task_kind == "reference_extraction" {
            return binding
                .service
                .persist_selection_task_evidence(
                    self.store.clone(),
                    binding.package_key.clone(),
                    run.id.clone(),
                )
                .await;
        }
        binding
            .service
            .persist_page_task_evidence(
                self.store.clone(),
                binding.package_key.clone(),
                run.id.clone(),
            )
            .await
    }
    async fn prepare_model(&self) -> Result<LlmModelSelection, AiError> {
        Ok(LlmModelSelection {
            provider_id: self.run.provider_id.clone(),
            model_id: self.run.model_id.clone(),
            reasoning_effort: self.run.reasoning_effort.clone(),
        })
    }
    async fn generate(
        &self,
        model: &LlmModelSelection,
        hash: Option<String>,
        messages: Vec<GenerationMessage>,
    ) -> Result<GenerationResponse, AiError> {
        let started = Instant::now();
        let stage = self.begin("generating", hash).await?;
        let prepared = self.execution.prepare_model(model).await?;
        let response = self
            .execution
            .execute(
                prepared,
                provider_messages_input(&messages),
                Some((
                    Uuid::parse_str(self.run.shared_conversation_id.as_ref().unwrap()).unwrap(),
                    Uuid::parse_str(self.run.shared_request_id.as_ref().unwrap()).unwrap(),
                )),
                Some(self.signal.clone()),
            )
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                self.fail(&stage, &error, started).await?;
                return Err(error);
            }
        };
        self.complete(&stage, Some(hash_text(&response.content)), started)
            .await?;
        Ok(GenerationResponse {
            model_id: model.model_id.clone(),
            content: response.content,
            usage: ProviderUsage {
                input_tokens: response
                    .input_tokens
                    .map(|value| u64::try_from(value).unwrap()),
                output_tokens: response
                    .output_tokens
                    .map(|value| u64::try_from(value).unwrap()),
                cache_hit_tokens: None,
                cache_miss_tokens: None,
            },
        })
    }
    async fn persist_candidate(
        &self,
        run: &str,
        bundle: crate::ai::candidate_contracts::AiEvidenceBundle,
        content: String,
        _retry: bool,
    ) -> Result<String, AiError> {
        let binding = self.store.native_note_trace().unwrap();
        binding
            .service
            .save_analysis_candidate(
                self.store.clone(),
                binding.package_key.clone(),
                run.into(),
                bundle,
                content,
            )
            .await
            .map(|candidate| candidate.payload_hash().into())
    }
    async fn finish(&self, usage: &ProviderUsage) -> Result<(), AiError> {
        let run = self.run.id.clone();
        let usage = (
            i64::try_from(usage.input_tokens.unwrap_or(0)).unwrap(),
            i64::try_from(usage.output_tokens.unwrap_or(0)).unwrap(),
            0,
            0,
        );
        self.data(move |data| data.finish(&run, "needs_review", None, None, Some(usage)))
            .await
    }
}
