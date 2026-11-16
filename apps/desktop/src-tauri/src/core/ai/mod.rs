#[path = "agent_configuration.rs"]
pub(crate) mod agent_configuration;
#[path = "attention_feedback_repository.rs"]
mod attention_feedback_repository;
#[path = "bounded_text_once.rs"]
mod bounded_text_once;
#[cfg(feature = "development-data")]
#[path = "calibration_runner.rs"]
mod calibration_runner;
#[path = "candidate_contracts.rs"]
pub(crate) mod candidate_contracts;
pub(crate) mod candidate_recovery;
#[path = "candidate_repository.rs"]
mod candidate_repository;
#[path = "candidate_schema.rs"]
mod candidate_schema;
#[path = "candidate_summary_repository.rs"]
mod candidate_summary_repository;
#[path = "citation_assessment_repository.rs"]
mod citation_assessment_repository;
#[path = "codex_app_server.rs"]
mod codex_app_server;
#[path = "codex_model_catalog/mod.rs"]
pub(crate) mod codex_model_catalog;
#[path = "commands.rs"]
mod commands;
#[path = "confirmation_support.rs"]
pub(crate) mod confirmation_support;
#[path = "credential.rs"]
mod credential;
#[path = "docling_execution.rs"]
pub(crate) mod docling_execution;
#[path = "docling_generation_support.rs"]
pub(crate) mod docling_generation_support;
#[path = "embedding.rs"]
mod embedding;
#[path = "embedding_settings.rs"]
pub(crate) mod embedding_settings;
#[path = "generation_contracts.rs"]
pub(crate) mod generation_contracts;
#[path = "generation_hooks.rs"]
pub(crate) mod generation_hooks;
#[path = "generation_policy.rs"]
pub(crate) mod generation_policy;
#[path = "generation_storage_failure.rs"]
mod generation_storage_failure;
#[path = "generation_support.rs"]
pub(crate) mod generation_support;
#[path = "knowledge_contracts.rs"]
pub(crate) mod knowledge_contracts;
#[path = "paper_analysis_contracts.rs"]
pub(crate) mod paper_analysis_contracts;
#[path = "paper_reading_contracts.rs"]
pub(crate) mod paper_reading_contracts;
#[path = "paper_reading_source.rs"]
pub(crate) mod paper_reading_source;
#[path = "reading_contracts.rs"]
pub(crate) mod reading_contracts;
#[path = "reading_support.rs"]
pub(crate) mod reading_support;
#[path = "research_input_contracts.rs"]
pub(crate) mod research_input_contracts;
#[path = "structured_generation_support.rs"]
pub(crate) mod structured_generation_support;
#[path = "task_evidence_support.rs"]
pub(crate) mod task_evidence_support;
#[path = "template_facts.rs"]
pub(crate) mod template_facts;
pub(crate) use embedding::{EmbeddingAccess, EmbeddingCallError, PreparedEmbedding};
#[path = "analysis_source.rs"]
pub(crate) mod analysis_source;
#[path = "analysis_support.rs"]
pub(crate) mod analysis_support;
#[path = "error.rs"]
pub(crate) mod error;
#[path = "evidence_repository.rs"]
mod evidence_repository;
#[cfg(test)]
#[path = "legacy_binding_fault_tests.rs"]
mod legacy_binding_fault_tests;
#[path = "llm_configuration.rs"]
mod llm_configuration;
#[path = "model.rs"]
mod model;
#[path = "model_metadata.rs"]
mod model_metadata;
#[path = "overview_confirmation_repository.rs"]
mod overview_confirmation_repository;
#[path = "overview_evidence_repository.rs"]
mod overview_evidence_repository;
pub(crate) mod overview_execution;
#[path = "overview_profile_repository.rs"]
mod overview_profile_repository;
#[path = "overview_retry_repository.rs"]
mod overview_retry_repository;
#[path = "overview_task.rs"]
mod overview_task;
mod page_evidence_storage;
#[path = "paper_context/mod.rs"]
pub(crate) mod paper_context;
#[path = "paper_generation.rs"]
mod paper_generation;
#[path = "paper_generation_admission.rs"]
pub(crate) mod paper_generation_admission;
#[path = "paper_overview_docling.rs"]
mod paper_overview_docling;
#[path = "paper_portrait.rs"]
mod paper_portrait;
#[allow(dead_code)]
#[path = "paper_reading.rs"]
mod paper_reading;
#[allow(dead_code)]
#[path = "paper_reading_bootstrap.rs"]
mod paper_reading_bootstrap;
#[allow(dead_code)]
#[path = "paper_reading_provider_input.rs"]
mod paper_reading_provider_input;
#[path = "paper_reading_shared_transport.rs"]
mod paper_reading_shared_transport;
#[path = "paper_template.rs"]
mod paper_template;
#[path = "portrait_conversations.rs"]
mod portrait_conversations;
pub(super) mod portrait_run;
#[path = "resource_template.rs"]
mod resource_template;
mod selection_evidence_storage;
#[path = "structured_generation.rs"]
mod structured_generation;
#[cfg(test)]
#[path = "structured_resource_upgrade_tests.rs"]
mod structured_resource_upgrade_tests;
#[path = "structured_sources.rs"]
mod structured_sources;
pub(crate) use structured_generation::seed_existing_context as seed_existing_structured_context;
pub(crate) use structured_sources::{
    authorized_pages, freeze_structured_sources, frozen_value, restrict_items,
};
mod generic_review;
pub(crate) mod item_review;
#[path = "provider/mod.rs"]
mod provider;
mod research_execution;
#[path = "research_task.rs"]
mod research_task;
#[path = "review_repository.rs"]
mod review_repository;
#[path = "review_support.rs"]
pub(crate) mod review_support;
#[path = "run_history.rs"]
pub(crate) mod run_history;
#[path = "run_manager.rs"]
mod run_manager;
#[path = "run_repository.rs"]
mod run_repository;
#[path = "run_state_repository.rs"]
mod run_state_repository;
#[path = "run_tasks.rs"]
mod run_tasks;
#[path = "settings_repository.rs"]
mod settings_repository;
#[path = "shared_responses.rs"]
mod shared_responses;
#[path = "structured_resource_validation.rs"]
pub(crate) mod structured_resource_validation;
#[path = "suggestion_context_repository.rs"]
mod suggestion_context_repository;
mod suggestion_execution;
#[path = "suggestion_review_repository.rs"]
mod suggestion_review_repository;
#[path = "suggestion_task.rs"]
mod suggestion_task;
#[path = "word_explanation.rs"]
pub(crate) mod word_explanation;
#[path = "word_explanation_repository.rs"]
pub(crate) mod word_explanation_repository;

pub(crate) use agent_configuration::AgentModelResolver;
#[cfg(feature = "development-data")]
pub use calibration_runner::{CalibrationPricing, CalibrationRunOptions, run_calibration};
pub(crate) use codex_app_server::CodexAppServerManager;
pub use commands::{
    cancel_ai_run, cancel_chatgpt_login, create_ai_candidate_summary_version, delete_ai_credential,
    discard_other_ai_candidate_summary_versions, fetch_llm_provider_models,
    get_ai_credential_summary, get_ai_run, get_chatgpt_account_status, get_llm_configuration,
    list_ai_candidates, list_ai_provider_overviews, list_ai_runs, logout_chatgpt, preview_ai_task,
    record_ai_attention_feedback, record_ai_citation_assessment, record_ai_evidence_inspection,
    revise_ai_candidate, save_llm_model_routes, save_llm_provider_config,
    select_ai_candidate_summary_version, set_ai_credential, start_ai_run, start_chatgpt_login,
    test_ai_provider,
};
pub(crate) use credential::{JsonCredentialStore, ProviderSecret};
pub(crate) use llm_configuration::validate_base_url;

#[cfg(all(test, debug_assertions, feature = "development-data"))]
pub(crate) use provider::EmbeddingHttpWire;
pub(crate) use provider::{CancellationSignal, ProviderError, ProviderErrorKind};
pub(crate) use run_manager::AiRunManager;

pub(crate) fn reconcile_legacy_runs_on_startup(
    guard: &mut crate::data_interface::DatabaseGuard<'_>,
) -> Result<usize, research_agent_core::conversation::StoreError> {
    crate::agent::repository::legacy_ai_runs::reconcile_on_startup_guarded(guard).map_err(|error| {
        if matches!(error, error::AiError::RunTerminalUnknown { .. }) {
            research_agent_core::conversation::StoreError::Persistence {
                diagnostic: format!("{}: {}", error.code(), error.user_message()),
            }
        } else {
            research_agent_core::conversation::StoreError::Unavailable
        }
    })
}

#[cfg(test)]
pub(crate) fn finish_reconciled_run(
    connection: &mut rusqlite::Connection,
    run_id: &str,
    status: &str,
) -> Result<(), error::AiError> {
    run_state_repository::finish_run(
        connection,
        run_id,
        status,
        Some("reconciled_shared_turn"),
        Some("共享 Agent 回合已完成，旧运行状态已恢复"),
        None,
    )
}

pub(crate) fn finish_reconciled_run_guarded(
    guard: &mut crate::data_interface::DatabaseGuard<'_>,
    run_id: &str,
    status: &str,
) -> Result<(), error::AiError> {
    run_state_repository::finish_run_guarded(
        guard,
        run_id,
        status,
        Some("reconciled_shared_turn"),
        Some("共享 Agent 回合已完成，旧运行状态已恢复"),
        None,
    )
}

struct EmbeddingProviderConfiguration {
    base_url: String,
    model_id: String,
    timeout_ms: i64,
    credential: ProviderSecret,
}

fn resolve_embedding_provider_configuration(
    connection: &rusqlite::Connection,
    credentials: &JsonCredentialStore,
) -> Result<EmbeddingProviderConfiguration, error::AiError> {
    let settings =
        settings_repository::get_provider_settings(connection, credentials, "qwen-embedding")?;
    if !settings.enabled || settings.provider_kind != "embedding" {
        return Err(error::AiError::InvalidInput(
            "请先启用 embedding provider".to_owned(),
        ));
    }
    Ok(EmbeddingProviderConfiguration {
        base_url: settings.base_url,
        model_id: settings.model_id,
        timeout_ms: settings.timeout_ms,
        credential: credentials.resolve("qwen-embedding")?,
    })
}

#[cfg(test)]
#[path = "candidate_persistence_tests.rs"]
mod candidate_persistence_tests;
mod matrix_evidence_storage;
#[path = "matrix_synthesis_context_repository.rs"]
mod matrix_synthesis_context_repository;
#[path = "matrix_synthesis_evidence_repository.rs"]
mod matrix_synthesis_evidence_repository;
#[cfg(test)]
#[path = "overview_stress_tests.rs"]
mod overview_stress_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub(crate) mod paper_item_review;

#[cfg(test)]
mod candidate_test_fixture;


#[cfg(test)]
pub(crate) use structured_generation::paper_grants as test_paper_grants;
