//! Selected literature decisions use the original Core stage journal and Agent transport.
use super::*;
use crate::{
    ai::{
        credential::JsonCredentialStore,
        docling_generation_support::StageCompletion,
        error::AiCommandError,
        generation_support::{AiRun, hash_text},
        paper_analysis_contracts::DoclingPaperOverviewDraft,
        paper_reading_contracts::PrepareRequest,
        run_state_repository,
    },
    data_interface::DatabaseGuard,
    runtime_plugins::literature_trace::service::LiteratureTraceService,
};
use research_workbench_native_sdk::{
    analysis_workflow::{
        docling::DoclingPromptInputV1, overview_task::OverviewTaskInputV1, wire::*,
    },
    candidate_contracts::EvidenceQuote,
};
use std::time::Instant;

pub(crate) struct NativeDoclingHost {
    pub store: AppStore,
    pub credentials: JsonCredentialStore,
    pub service: Arc<LiteratureTraceService>,
    pub key: String,
    pub execution: DoclingExecution,
}
impl NativeDoclingHost {
    async fn validate(&self, input: WorkflowInputV1) -> Result<(), AiError> {
        let (decision, origin) = self
            .service
            .analysis_workflow_decision(&self.key, input)
            .await
            .map_err(|failure| AiError::NativeWorkflow {
                code: "native_workflow_failed",
                failure,
            })?;
        let error = match decision {
            WorkflowDecisionV1::Validated { rejection: None } => return Ok(()),
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => AiError::from(error),
            _ => AiError::Provider("plugin_host_response_mismatch"),
        };
        let mut failure = crate::runtime_plugins::native::NativeHostError::for_profile(
            crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
            error.code(),
            error.to_string(),
        )
        .package(&self.key);
        failure.instance_id = Some(origin.instance_id);
        failure.request_id = Some(origin.native_request_id);
        Err(AiError::NativeWorkflow {
            code: error.code(),
            failure,
        })
    }
}
impl DoclingHost for NativeDoclingHost {
    async fn prepare(
        &self,
        draft: DoclingPaperOverviewDraft,
    ) -> Result<PreparedOverviewRun, AiCommandError> {
        self.validate(WorkflowInputV1::ValidateDoclingStart {
            revision: draft.overview_profile_selection_revision,
            has_template: draft.paper_template_ref.is_some(),
        })
        .await?;
        let (prepared, snapshot, assets) = if let Some(snapshot) = draft.context_snapshot_id.clone()
        {
            self.service
                .load_docling_snapshot(
                    self.store.clone(),
                    self.key.clone(),
                    draft.paper_id.clone(),
                    snapshot,
                )
                .await?
        } else {
            self.service
                .prepare_docling_context(
                    self.store.clone(),
                    self.key.clone(),
                    PrepareRequest {
                        paper_id: draft.paper_id.clone(),
                        attachment_ids: draft.attachment_ids.clone(),
                        image_assets: draft.image_assets.clone(),
                    },
                )
                .await?
        };
        self.validate(WorkflowInputV1::ValidateDoclingSources {
            statuses: prepared
                .attachment_statuses
                .iter()
                .map(|status| status.parse_status.clone())
                .collect(),
            picture_count: prepared.picture_item_count,
        })
        .await?;
        let mut task = self
            .service
            .build_analysis_overview_task(
                &self.key,
                OverviewTaskInputV1 {
                    paper_id: draft.paper_id,
                    attachment_ids: draft.attachment_ids,
                    selection_revision: draft.overview_profile_selection_revision,
                    template: draft.paper_template_ref,
                    model_selection: draft.model_selection,
                },
            )
            .await?;
        let preview = self
            .service
            .preview_analysis_overview(
                self.store.clone(),
                self.credentials.clone(),
                &self.key,
                task.clone(),
            )
            .await?;
        task.confirmed_scope_hash = Some(preview.scope_hash);
        if preview.requires_cloud_consent {
            task.cloud_consent_version = Some("ai-cloud-consent-v1".into());
        }
        let shared = match (draft.shared_conversation_id, draft.shared_request_id) {
            (Some(conversation), Some(request)) => Some((conversation, request)),
            (None, None) => None,
            _ => return Err(AiError::InvalidInput("结构化阅读共享提交身份不完整".into()).into()),
        };
        let run = self
            .service
            .admit_analysis_overview(
                self.store.clone(),
                self.credentials.clone(),
                self.key.clone(),
                task,
                shared,
            )
            .await?;
        let run_id = run.id.clone();
        journal(&self.store, move |db| {
            db.execute(
                "update ai_runs set context_snapshot_id = ?2 where id = ?1",
                rusqlite::params![run_id, snapshot.snapshot_id],
            )?;
            Ok(())
        })
        .await?;
        let retrieval_started = Instant::now();
        let run_id = run.id.clone();
        let scope = run.scope_hash.clone();
        let retrieval_stage = journal(&self.store, move |db| {
            run_state_repository::begin_stage(db, &run_id, "retrieving", Some(&scope))
        })
        .await?;
        let bundle = self
            .service
            .persist_analysis_docling_evidence(
                self.store.clone(),
                self.key.clone(),
                run.clone(),
                prepared.context_items.clone(),
                assets.clone(),
            )
            .await?;
        let output = hash_text(&serde_json::to_string(&bundle).unwrap_or_default());
        journal(&self.store, move |db| {
            run_state_repository::complete_stage(
                db,
                &retrieval_stage,
                StageCompletion {
                    output_hash: Some(&output),
                    input_tokens: None,
                    output_tokens: None,
                    duration_ms: elapsed_ms(retrieval_started),
                },
            )
        })
        .await?;
        let generation_started = Instant::now();
        let (input, _) = self
            .service
            .render_docling_overview_prompt(
                &self.key,
                DoclingPromptInputV1 {
                    items: prepared.context_items,
                    assets,
                    profile: run.overview_profile.clone(),
                    evidence: bundle
                        .items
                        .iter()
                        .map(|item| EvidenceQuote {
                            citation_key: item.citation_key.clone(),
                            quote_exact: item.quote_exact.clone(),
                        })
                        .collect(),
                },
            )
            .await?;
        let run_id = run.id.clone();
        let hash = hash_text(&input.audit_text);
        let generation_stage_id = journal(&self.store, move |db| {
            run_state_repository::begin_stage(db, &run_id, "generating", Some(&hash))
        })
        .await?;
        Ok(PreparedOverviewRun {
            state: PreparedOverviewRunState {
                run,
                generation_stage_id,
                generation_started,
                bundle,
            },
            selection: LlmModelSelection {
                provider_id: preview.provider_id,
                model_id: preview.model_id,
                reasoning_effort: preview.reasoning_effort,
            },
            input,
        })
    }
    async fn prepare_model(
        &self,
        selection: &LlmModelSelection,
    ) -> Result<PreparedAgentModel, AiError> {
        self.execution.prepare_model(selection).await
    }
    async fn execute(
        &self,
        model: PreparedAgentModel,
        input: ProviderInput,
        shared: Option<(uuid::Uuid, uuid::Uuid)>,
        cancellation: Option<CancellationSignal>,
    ) -> Result<PaperGenerationResponse, AiError> {
        self.execution
            .execute(model, input, shared, cancellation)
            .await
    }
    async fn success(
        &self,
        state: PreparedOverviewRunState,
        response: PaperGenerationResponse,
    ) -> Result<AiRun, AiCommandError> {
        self.finish_success(state, response)
            .await
            .map_err(Into::into)
    }
    async fn failure(
        &self,
        state: PreparedOverviewRunState,
        error: AiError,
    ) -> Result<AiRun, AiCommandError> {
        if outcome_unknown(&error) {
            return Err(error.into());
        }
        let stage = state.generation_stage_id;
        let code = error.code();
        let started = state.generation_started;
        let _ = journal(&self.store, move |db| {
            run_state_repository::fail_stage(
                db,
                &stage,
                code,
                "结构化阅读生成失败；未保存 provider 原始响应",
                elapsed_ms(started),
            )
        })
        .await;
        finish_failed(&self.store, state.run.id, &error).await?;
        Err(error.into())
    }
}

pub(super) async fn journal<T: Send + 'static>(
    store: &AppStore,
    operation: impl FnOnce(&mut DatabaseGuard<'_>) -> Result<T, AiError> + Send + 'static,
) -> Result<T, AiError> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || operation(&mut store.connection()?))
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
}
pub(super) fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}
pub(super) fn outcome_unknown(error: &AiError) -> bool {
    matches!(
        error.code(),
        "data_operation_outcome_unknown" | "ai_run_terminal_outcome_unknown"
    )
}
pub(super) async fn finish_failed(
    store: &AppStore,
    run: String,
    error: &AiError,
) -> Result<(), AiError> {
    let status = if matches!(error, AiError::Canceled) {
        "canceled"
    } else {
        "failed"
    };
    let code = error.code();
    journal(store, move |db| {
        run_state_repository::finish_run_guarded(
            db,
            &run,
            status,
            Some(code),
            Some("结构化阅读 AI 提取失败"),
            None,
        )
    })
    .await
}
