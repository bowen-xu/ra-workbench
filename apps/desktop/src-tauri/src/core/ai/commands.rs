use tauri::{AppHandle, State};

use crate::agent::model::AgentModelResolver;
use crate::storage::AppStore;

use super::{
    AiRunManager, CodexAppServerManager, attention_feedback_repository, candidate_repository,
    candidate_summary_repository, citation_assessment_repository,
    credential::{CredentialSummary, JsonCredentialStore},
    error::{AiCommandError, AiError},
    llm_configuration,
    model::{
        AiAttentionFeedback, AiAttentionFeedbackDraft, AiCandidate, AiCandidateRevisionDraft,
        AiCandidateSummaryDiscardDraft, AiCandidateSummarySelectionDraft,
        AiCandidateSummaryVersionDraft, AiCitationAssessmentDraft, AiProviderOverview,
        AiProviderTestResult, AiRun, AiTaskPreview, AiTaskSpecDraft, ChatGptAccountStatus,
        ChatGptLoginStart, LlmConfiguration, LlmModelRouteDraft, LlmProviderConfigDraft,
        LlmProviderModels,
    },
    review_repository,
    run_manager::test_provider_connection,
    run_repository, run_state_repository, settings_repository,
};

#[tauri::command]
pub fn get_llm_configuration(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
) -> Result<LlmConfiguration, AiCommandError> {
    let connection = store.connection()?;
    llm_configuration::get_configuration(&connection, &credentials).map_err(Into::into)
}

#[tauri::command]
pub fn save_llm_provider_config(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    draft: LlmProviderConfigDraft,
) -> Result<LlmConfiguration, AiCommandError> {
    let connection = store.connection()?;
    llm_configuration::save_provider(&connection, &credentials, draft).map_err(Into::into)
}

#[tauri::command]
pub fn save_llm_model_routes(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    drafts: Vec<LlmModelRouteDraft>,
) -> Result<LlmConfiguration, AiCommandError> {
    let mut connection = store.connection()?;
    llm_configuration::save_routes(&mut connection, &credentials, drafts).map_err(Into::into)
}

#[tauri::command]
pub async fn fetch_llm_provider_models(
    app: AppHandle,
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    provider_id: String,
    models: State<'_, AgentModelResolver>,
    refresh: Option<bool>,
) -> Result<LlmProviderModels, AiCommandError> {
    if provider_id == "openai" {
        return models
            .chatgpt_models(refresh.unwrap_or(false))
            .await
            .map_err(Into::into);
    }
    let store = store.inner().clone();
    let credentials = credentials.inner().clone();
    let credentials_for_load = credentials.clone();
    let (provider, credential) = tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        llm_configuration::provider_for_model_fetch(
            &connection,
            &credentials_for_load,
            &provider_id,
        )
    })
    .await
    .map_err(|_| AiCommandError::from(AiError::TemporaryUnavailable))??;
    let models = llm_configuration::fetch_models(provider, credential).await?;
    Ok(super::model_metadata::enrich_models(&app, models).await)
}

#[tauri::command]
pub async fn get_chatgpt_account_status(
    codex: State<'_, CodexAppServerManager>,
) -> Result<ChatGptAccountStatus, AiCommandError> {
    match codex.account_status().await {
        Ok(status) => Ok(status),
        Err(AiError::InvalidInput(_)) | Err(AiError::TemporaryUnavailable) => {
            Ok(ChatGptAccountStatus {
                runtime_available: false,
                configured: false,
                auth_type: String::new(),
                email: None,
                plan_type: None,
            })
        }
        Err(error) => Err(error.into()),
    }
}

#[tauri::command]
pub async fn start_chatgpt_login(
    codex: State<'_, CodexAppServerManager>,
) -> Result<ChatGptLoginStart, AiCommandError> {
    codex.start_login().await.map_err(Into::into)
}

#[tauri::command]
pub async fn cancel_chatgpt_login(
    codex: State<'_, CodexAppServerManager>,
    login_id: String,
) -> Result<bool, AiCommandError> {
    codex
        .cancel_login(&login_id)
        .await
        .map(|_| true)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn logout_chatgpt(
    codex: State<'_, CodexAppServerManager>,
    models: State<'_, AgentModelResolver>,
) -> Result<bool, AiCommandError> {
    codex.logout().await?;
    models.invalidate_chatgpt_models().await;
    Ok(true)
}

#[tauri::command]
pub fn get_ai_credential_summary(
    credentials: State<'_, JsonCredentialStore>,
    provider_id: String,
) -> Result<CredentialSummary, AiCommandError> {
    credentials.summary(&provider_id).map_err(Into::into)
}

#[tauri::command]
pub fn set_ai_credential(
    credentials: State<'_, JsonCredentialStore>,
    provider_id: String,
    api_key: String,
) -> Result<CredentialSummary, AiCommandError> {
    credentials.set(&provider_id, &api_key).map_err(Into::into)
}

#[tauri::command]
pub fn delete_ai_credential(
    credentials: State<'_, JsonCredentialStore>,
    provider_id: String,
) -> Result<CredentialSummary, AiCommandError> {
    credentials.delete(&provider_id).map_err(Into::into)
}

#[tauri::command]
pub fn list_ai_provider_overviews(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
) -> Result<Vec<AiProviderOverview>, AiCommandError> {
    let connection = store.connection()?;
    settings_repository::list_provider_overviews(&connection, &credentials).map_err(Into::into)
}

#[tauri::command]
pub async fn test_ai_provider(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    models: State<'_, AgentModelResolver>,
    provider_id: String,
) -> Result<AiProviderTestResult, AiCommandError> {
    let store = store.inner().clone();
    let credentials = credentials.inner().clone();
    let provider_id_for_load = provider_id.clone();
    let credentials_for_load = credentials.clone();
    let settings = tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        settings_repository::get_provider_settings(
            &connection,
            &credentials_for_load,
            &provider_id_for_load,
        )
    })
    .await
    .map_err(|_| AiCommandError::from(AiError::TemporaryUnavailable))??;
    test_provider_connection(settings, &credentials, models.inner())
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn preview_ai_task(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    draft: AiTaskSpecDraft,
) -> Result<AiTaskPreview, AiCommandError> {
    let store = store.inner().clone();
    let credentials = credentials.inner().clone();
    if matches!(
        draft.task_kind.as_str(),
        "paper_profile_classification"
            | "paper_suggestions"
            | "reference_extraction"
            | "matrix_synthesis"
    ) && let Some(binding) = store.native_note_trace().cloned()
    {
        return binding
            .service
            .preview_analysis_overview(store, credentials, &binding.package_key, draft)
            .await
            .map_err(Into::into);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        run_repository::preview_task(
            store.require_generation_policy_source()?,
            &connection,
            &credentials,
            &draft,
        )
        .map_err(Into::into)
    })
    .await
    .map_err(|_| AiCommandError::from(AiError::TemporaryUnavailable))?
}

#[tauri::command]
pub async fn start_ai_run(
    app: AppHandle,
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    manager: State<'_, AiRunManager>,
    draft: AiTaskSpecDraft,
) -> Result<AiRun, AiCommandError> {
    if !is_supported_ai_task(&draft.task_kind) {
        return Err(AiError::InvalidInput("当前里程碑尚不支持执行该 AI 任务".to_owned()).into());
    }
    if draft.task_kind == "paper_profile_classification" && store.native_note_trace().is_some() {
        return super::portrait_run::start(
            app,
            store.inner().clone(),
            credentials.inner().clone(),
            manager.inner().clone(),
            draft,
        )
        .await
        .map_err(Into::into);
    }
    if matches!(
        draft.task_kind.as_str(),
        "paper_suggestions" | "reference_extraction" | "matrix_synthesis"
    ) && store.native_note_trace().is_some()
    {
        return super::suggestion_task::start(
            app,
            store.inner().clone(),
            credentials.inner().clone(),
            manager.inner().clone(),
            draft,
        )
        .await
        .map_err(Into::into);
    }
    if draft.task_kind == "paper_profile_classification" {
        let preview = tauri::async_runtime::spawn_blocking({
            let store = store.inner().clone();
            let credentials = credentials.inner().clone();
            let draft = draft.clone();
            move || -> Result<AiTaskPreview, AiError> {
                let connection = store.connection()?;
                let preview = run_repository::preview_task(
                    store.require_generation_policy_source()?,
                    &connection,
                    &credentials,
                    &draft,
                )?;
                if draft.confirmed_scope_hash.as_deref() != Some(preview.scope_hash.as_str()) {
                    return Err(AiError::InvalidInput(
                        "任务范围已变化，请重新确认外发预览".into(),
                    ));
                }
                if preview.requires_cloud_consent
                    && draft.cloud_consent_version.as_deref() != Some("ai-cloud-consent-v1")
                {
                    return Err(AiError::InvalidInput(
                        "启动云端任务前必须确认本次外发范围".into(),
                    ));
                }
                Ok(preview)
            }
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)??;
        let paper = preview
            .scope_items
            .first()
            .and_then(|scope| scope.parent_object_id.as_deref())
            .ok_or_else(|| AiError::InvalidInput("画像任务缺少所属文献".into()))?;
        let files = preview
            .scope_items
            .iter()
            .map(|scope| scope.object_id.clone())
            .collect::<Vec<_>>();
        super::paper_generation::prepare_sources(&store, paper, &files).await?;
    }
    let run = tauri::async_runtime::spawn_blocking({
        let store = store.inner().clone();
        let credentials = credentials.inner().clone();
        move || -> Result<AiRun, AiError> {
            let mut connection = store.connection()?;
            run_repository::insert_run_guarded(
                store.require_paper_generation_source()?,
                store.require_generation_policy_source()?,
                &mut connection,
                &credentials,
                &draft,
                None,
            )
        }
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)??;
    register_and_spawn(
        app,
        store.inner().clone(),
        credentials.inner().clone(),
        manager.inner().clone(),
        run,
    )
    .await
    .map_err(Into::into)
}

pub(super) async fn register_and_spawn(
    app: AppHandle,
    store: AppStore,
    credentials: JsonCredentialStore,
    manager: AiRunManager,
    run: AiRun,
) -> Result<AiRun, AiError> {
    let signal = match manager.register(&run.id) {
        Ok(signal) => signal,
        Err(error) => {
            let store = store.clone();
            let run_id = run.id.clone();
            let code = error.code();
            tauri::async_runtime::spawn_blocking(move || -> Result<(), AiError> {
                let mut connection = store.connection()?;
                run_state_repository::finish_run_guarded(
                    &mut connection,
                    &run_id,
                    "failed",
                    Some(code),
                    Some("该 AI 运行已启动或运行 ID 无效"),
                    None,
                )?;
                Ok(())
            })
            .await
            .map_err(|_| AiError::TemporaryUnavailable)??;
            return Err(error);
        }
    };
    manager.spawn(app, store, credentials, run.clone(), signal);
    Ok(run)
}

fn is_supported_ai_task(task_kind: &str) -> bool {
    task_kind == "provider_diagnostic" || run_repository::uses_shared_agent_compatibility(task_kind)
}

#[cfg(test)]
mod tests {
    use super::is_supported_ai_task;

    #[test]
    fn production_entrypoint_allows_shared_research_tasks() {
        for task_kind in [
            "provider_diagnostic",
            "selection_explanation",
            "paper_overview",
            "paper_suggestions",
            "reference_extraction",
            "matrix_synthesis",
        ] {
            assert!(is_supported_ai_task(task_kind), "{task_kind}");
        }
        assert!(!is_supported_ai_task("paper_concept_map"));
        assert!(!is_supported_ai_task("unknown"));
    }
}

#[tauri::command]
pub async fn list_ai_candidates(
    store: State<'_, AppStore>,
    project_id: Option<String>,
    paper_id: Option<String>,
) -> Result<Vec<AiCandidate>, AiCommandError> {
    let store = store.inner().clone();
    // Keep both DB lock acquisition and history assembly off the UI thread.
    // Making this command synchronous, or acquiring the connection before the
    // blocking worker, can reintroduce stalls whenever a literature row is selected.
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        candidate_repository::list_candidates(
            &connection,
            project_id.as_deref(),
            paper_id.as_deref(),
        )
        .map_err(AiCommandError::from)
    })
    .await
    .map_err(|_| AiCommandError::from(AiError::TemporaryUnavailable))?
}

#[tauri::command]
pub async fn revise_ai_candidate(
    store: State<'_, AppStore>,
    draft: AiCandidateRevisionDraft,
) -> Result<AiCandidate, AiCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .revise_analysis_candidate(store, binding.package_key, draft)
            .await
            .map_err(Into::into);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut connection = store.connection()?;
        candidate_repository::revise_candidate(
            store.require_generation_policy_source()?,
            &mut connection,
            draft,
        )
        .map_err(Into::into)
    })
    .await
    .map_err(|_| AiCommandError::from(AiError::TemporaryUnavailable))?
}

#[tauri::command]
pub async fn record_ai_citation_assessment(
    store: State<'_, AppStore>,
    draft: AiCitationAssessmentDraft,
) -> Result<AiCandidate, AiCommandError> {
    let store = store.inner().clone();
    if let Some(binding) = store.native_note_trace().cloned() {
        return binding
            .service
            .record_candidate_citation_assessment(store, binding.package_key, draft)
            .await
            .map_err(Into::into);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        citation_assessment_repository::record_citation_assessment(
            store.require_generation_policy_source()?,
            &connection,
            draft,
        )
        .map_err(Into::into)
    })
    .await
    .map_err(|_| AiCommandError::from(AiError::TemporaryUnavailable))?
}

#[tauri::command]
pub fn record_ai_evidence_inspection(
    store: State<'_, AppStore>,
    candidate_id: String,
    candidate_revision: i64,
    evidence_item_id: String,
) -> Result<bool, AiCommandError> {
    let connection = store.connection()?;
    review_repository::record_evidence_inspection(
        &connection,
        &candidate_id,
        candidate_revision,
        &evidence_item_id,
    )
    .map_err(Into::into)
}

#[tauri::command]
pub fn record_ai_attention_feedback(
    store: State<'_, AppStore>,
    draft: AiAttentionFeedbackDraft,
) -> Result<AiAttentionFeedback, AiCommandError> {
    let connection = store.connection()?;
    attention_feedback_repository::record_attention_feedback(
        store.require_generation_policy_source()?,
        &connection,
        draft,
    )
    .map_err(Into::into)
}

#[tauri::command]
pub fn create_ai_candidate_summary_version(
    store: State<'_, AppStore>,
    draft: AiCandidateSummaryVersionDraft,
) -> Result<AiCandidate, AiCommandError> {
    let mut connection = store.connection()?;
    candidate_summary_repository::create_summary_version(&mut connection, draft).map_err(Into::into)
}

#[tauri::command]
pub fn select_ai_candidate_summary_version(
    store: State<'_, AppStore>,
    draft: AiCandidateSummarySelectionDraft,
) -> Result<AiCandidate, AiCommandError> {
    let mut connection = store.connection()?;
    candidate_summary_repository::select_summary_version(&mut connection, draft).map_err(Into::into)
}

#[tauri::command]
pub fn discard_other_ai_candidate_summary_versions(
    store: State<'_, AppStore>,
    draft: AiCandidateSummaryDiscardDraft,
) -> Result<AiCandidate, AiCommandError> {
    let mut connection = store.connection()?;
    candidate_summary_repository::discard_other_summary_versions(&mut connection, draft)
        .map_err(Into::into)
}

#[tauri::command]
pub fn get_ai_run(
    store: State<'_, AppStore>,
    manager: State<'_, AiRunManager>,
    run_id: String,
) -> Result<AiRun, AiCommandError> {
    if let Some(error) = manager.terminal_unknown(&run_id) {
        return Err(error);
    }
    let connection = store.connection()?;
    run_repository::get_run(&connection, &run_id).map_err(Into::into)
}

#[tauri::command]
pub fn list_ai_runs(
    store: State<'_, AppStore>,
    limit: Option<i64>,
) -> Result<Vec<AiRun>, AiCommandError> {
    let connection = store.connection()?;
    run_repository::list_runs(&connection, limit.unwrap_or(25)).map_err(Into::into)
}

#[tauri::command]
pub fn cancel_ai_run(
    store: State<'_, AppStore>,
    manager: State<'_, AiRunManager>,
    run_id: String,
) -> Result<bool, AiCommandError> {
    let requested = {
        let connection = store.connection()?;
        run_state_repository::request_cancel(&connection, &run_id)?
    };
    let signaled = manager.cancel(&run_id)?;
    Ok(requested || signaled)
}
