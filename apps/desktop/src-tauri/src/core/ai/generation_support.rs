//! Existing run stages and model execution, closed over the original owners.
//! This capability does not select evidence, templates or candidate policy.
use std::{sync::atomic::AtomicU64, time::Instant};

use tauri::AppHandle;

use super::{
    credential::JsonCredentialStore,
    error::AiError,
    model::AiProviderSettings,
    run_manager,
    run_tasks::{self, RunTaskContext},
};
use crate::storage::AppStore;

#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) use super::overview_execution::OverviewHost;
#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) use super::research_execution::execute_research_task;
#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) use super::suggestion_execution::execute_suggestions;

pub(crate) use super::candidate_repository::{
    GeneratedCandidateCommit, save_prepared_candidate_guarded,
};
pub(crate) use super::generation_storage_failure::GenerationStorageUnknown;
pub(crate) use super::overview_evidence_repository::preview_frozen_overview_evidence;

pub(crate) use super::overview_profile_repository::resolve_selection as resolve_overview_profile_selection;
pub(crate) use super::overview_retry_repository::validate_retry_shape as validate_overview_retry_shape;

pub(crate) use super::research_execution::policy_input as research_policy_input;
pub(crate) use super::run_manager::hash_text;
pub(crate) use super::run_repository::{
    RunCreateIdentities, insert_prepared_run_guarded, resolve_task_scope, scope_hash,
    validate_fixed_task_scope, validate_task_before_policy,
};
#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) use super::run_tasks::fail_task_stage;
#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) use super::shared_responses::provider_messages_input;

pub(crate) use super::{
    generation_contracts::{
        PaperGenerationResponse, ProviderInput, ProviderMessage, ProviderMessageRole,
        VisionGenerationResponse,
    },
    model::{AiRun, AiTaskPreview, AiTaskSpecDraft},
    provider::{GenerationMessage, GenerationResponse, GenerationRole, ProviderUsage},
};

pub(crate) fn validate_evidence_bundle(
    bundle: &super::candidate_contracts::AiEvidenceBundle,
    minimum: Option<usize>,
) -> Result<(), AiError> {
    run_tasks::validate_bundle(bundle, minimum)
}

/// Frozen settings remain opaque; neither credentials nor a provider client
/// enter the domain's prompt construction.
pub(crate) struct OverviewModel(AiProviderSettings);

pub(crate) struct OverviewExecution<'a> {
    context: RunTaskContext<'a>,
}

impl<'a> OverviewExecution<'a> {
    pub(crate) fn new(
        app: &'a AppHandle,
        store: &'a AppStore,
        credentials: &'a JsonCredentialStore,
        run: &'a AiRun,
        signal: &'a super::CancellationSignal,
        sequence: &'a AtomicU64,
    ) -> Self {
        Self {
            context: RunTaskContext {
                app,
                store,
                credentials,
                run,
                signal,
                sequence,
            },
        }
    }

    pub(crate) async fn resolve_stage(&self) -> Result<String, AiError> {
        run_tasks::complete_placeholder_stage(
            &self.context,
            "resolving_scope",
            Some(self.context.run.scope_hash.clone()),
        )
        .await
    }

    pub(crate) async fn begin(&self, kind: &str, input: Option<String>) -> Result<String, AiError> {
        run_manager::begin_stage(self.context.store, &self.context.run.id, kind, input).await
    }

    pub(crate) async fn complete(
        &self,
        stage: &str,
        output: Option<String>,
        started: Instant,
    ) -> Result<(), AiError> {
        run_manager::complete_stage(self.context.store, stage, output, None, started).await
    }

    pub(crate) async fn fail(
        &self,
        stage: &str,
        error: &AiError,
        started: Instant,
    ) -> Result<(), AiError> {
        run_tasks::fail_task_stage(self.context.store, stage, error, started).await
    }

    pub(crate) fn progress(&self) {
        run_manager::emit_progress(
            self.context.app,
            &self.context.run.id,
            self.context.sequence,
        );
    }

    pub(crate) async fn prepare_model(&self) -> Result<OverviewModel, AiError> {
        run_tasks::load_settings(
            self.context.store,
            self.context.credentials,
            &self.context.run.provider_id,
            &self.context.run.model_id,
        )
        .await
        .map(OverviewModel)
    }

    pub(crate) async fn generate(
        &self,
        model: &OverviewModel,
        input_hash: Option<String>,
        messages: Vec<GenerationMessage>,
    ) -> Result<GenerationResponse, AiError> {
        let request = run_tasks::request_for(self.context.run, &model.0, messages)?;
        run_tasks::generate(&self.context, &model.0, input_hash, &request).await
    }

    pub(crate) async fn finish(&self, usage: &ProviderUsage) -> Result<(), AiError> {
        run_tasks::finish_run(
            self.context.store,
            self.context.run,
            "needs_review",
            None,
            None,
            run_manager::usage_tuple(usage)?,
        )
        .await
    }
}

pub(crate) use super::evidence_repository::load_bundle;

#[cfg(test)]
pub(crate) use super::candidate_repository::save_prepared_candidate;

#[cfg(test)]
pub(crate) use super::paper_overview_docling::persist_docling_evidence as persist_test_docling_evidence;
