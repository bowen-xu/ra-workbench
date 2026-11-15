#[path = "run_manager/lifecycle.rs"]
mod lifecycle;
pub(crate) use lifecycle::AiRunManager;
use lifecycle::fail_stage;

use std::{
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use research_agent_core::{
    model::ModelError,
    resilience::{RetryDecision, RetryPolicy},
};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use crate::agent::model::{AgentHost, AgentModelResolver, AgentModelSelection};
use crate::agent::service::AgentSessionService;
use crate::storage::AppStore;

use super::{
    credential::JsonCredentialStore,
    error::AiError,
    model::{AiProviderSettings, AiProviderTestResult, AiRun, AiRunProgressEvent},
    provider::{
        CancellationSignal, EmbeddingProvider, EmbeddingRequest, FixtureGenerationProvider,
        GenerationMessage, GenerationProvider, GenerationRequest, GenerationResponse,
        GenerationResponseFormat, GenerationRole, ProviderError, ProviderErrorKind, ProviderUsage,
        QwenEmbeddingProvider, SharedResponsesGenerationProvider, ThinkingMode,
    },
    run_state_repository::{self, StageCompletion},
};

pub(crate) async fn test_provider_connection(
    settings: AiProviderSettings,
    credentials: &JsonCredentialStore,
    models: &AgentModelResolver,
) -> Result<AiProviderTestResult, AiError> {
    if !settings.enabled {
        return Err(AiError::InvalidInput("请先启用并保存 provider".to_owned()));
    }
    if settings.provider_kind == "embedding" {
        return test_embedding_provider(settings, credentials).await;
    }
    if settings.provider_kind != "generation" {
        return Err(AiError::InvalidInput("未知 provider 类型".to_owned()));
    }
    let request = GenerationRequest {
        model_id: settings.model_id.clone(),
        messages: vec![GenerationMessage {
            role: GenerationRole::User,
            content: "Return only {\"kind\":\"provider_diagnostic_v1\",\"ok\":true}. No research content is included.".to_owned(),
        }],
        max_output_tokens: 64,
        thinking_mode: ThinkingMode::Disabled,
        response_format: GenerationResponseFormat::JsonObject,
        timeout: Duration::from_millis(u64::try_from(settings.timeout_ms)
            .map_err(|_| AiError::InvalidInput("超时设置无效".to_owned()))?),
    };
    let signal = CancellationSignal::default();
    let response = generate_once(
        &settings.provider_id,
        &settings.base_url,
        credentials,
        Some(models.clone()),
        None,
        &request,
        &signal,
    )
    .await
    .map_err(provider_error)?;
    validate_diagnostic(&response.content)?;
    Ok(AiProviderTestResult {
        provider_id: settings.provider_id,
        model_id: response.model_id,
        reachable: true,
        message: "连通性验证通过；测试未包含文献或笔记内容".to_owned(),
    })
}

async fn test_embedding_provider(
    settings: AiProviderSettings,
    credentials: &JsonCredentialStore,
) -> Result<AiProviderTestResult, AiError> {
    if settings.provider_id != "qwen-embedding" {
        return Err(AiError::InvalidInput("未知 embedding provider".to_owned()));
    }
    let credential = credentials.resolve(&settings.provider_id)?;
    let provider =
        QwenEmbeddingProvider::new(&settings.base_url, credential).map_err(provider_error)?;
    let request = EmbeddingRequest {
        model_id: settings.model_id.clone(),
        inputs: vec!["provider connectivity check; no research content".to_owned()],
        dimension: 64,
        timeout: Duration::from_millis(
            u64::try_from(settings.timeout_ms)
                .map_err(|_| AiError::InvalidInput("超时设置无效".to_owned()))?,
        ),
    };
    let response = provider
        .embed(&request, &CancellationSignal::default())
        .await
        .map_err(provider_error)?;
    Ok(AiProviderTestResult {
        provider_id: settings.provider_id,
        model_id: response.model_id,
        reachable: true,
        message: "连通性验证通过；测试未包含文献、概念或笔记内容".to_owned(),
    })
}

async fn execute_run(
    app: &AppHandle,
    store: &AppStore,
    credentials: &JsonCredentialStore,
    run: &AiRun,
    signal: &CancellationSignal,
    sequence: &AtomicU64,
) -> Result<(), AiError> {
    match run.task_kind.as_str() {
        "provider_diagnostic" => {
            super::run_tasks::execute_diagnostic(app, store, credentials, run, signal, sequence)
                .await
        }
        "selection_explanation" => {
            super::run_tasks::execute_selection(app, store, credentials, run, signal, sequence)
                .await
        }
        "paper_overview" | "paper_profile_classification" => {
            super::overview_task::execute_overview(app, store, credentials, run, signal, sequence)
                .await
        }
        "paper_suggestions" => {
            super::suggestion_task::execute_suggestions(
                app,
                store,
                credentials,
                run,
                signal,
                sequence,
            )
            .await
        }
        "reference_extraction" | "matrix_synthesis" => {
            super::research_task::execute_research_task(
                app,
                store,
                credentials,
                run,
                signal,
                sequence,
            )
            .await
        }
        _ => Err(AiError::InvalidInput(
            "当前里程碑尚不支持执行该 AI 任务".to_owned(),
        )),
    }
}

pub(crate) fn emit_progress(app: &AppHandle, run_id: &str, sequence: &AtomicU64) {
    let event = AiRunProgressEvent {
        run_id: run_id.to_owned(),
        sequence: sequence.fetch_add(1, Ordering::AcqRel) + 1,
    };
    let _ = app.emit("ai-run-progress", event);
}

pub(crate) async fn begin_stage(
    store: &AppStore,
    run_id: &str,
    kind: &str,
    input_hash: Option<String>,
) -> Result<String, AiError> {
    let store = store.clone();
    let run_id = run_id.to_owned();
    let kind = kind.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let mut connection = store.connection()?;
        run_state_repository::begin_stage(&mut connection, &run_id, &kind, input_hash.as_deref())
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
}

pub(crate) async fn complete_stage(
    store: &AppStore,
    stage_id: &str,
    output_hash: Option<String>,
    usage: Option<&ProviderUsage>,
    started: Instant,
) -> Result<(), AiError> {
    let store = store.clone();
    let stage_id = stage_id.to_owned();
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let input_tokens = usage
        .and_then(|value| value.input_tokens)
        .and_then(|value| i64::try_from(value).ok());
    let output_tokens = usage
        .and_then(|value| value.output_tokens)
        .and_then(|value| i64::try_from(value).ok());
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection()?;
        run_state_repository::complete_stage(
            &connection,
            &stage_id,
            StageCompletion {
                output_hash: output_hash.as_deref(),
                input_tokens,
                output_tokens,
                duration_ms,
            },
        )
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
}

pub(crate) struct GenerationStageContext<'a> {
    pub(crate) app: &'a AppHandle,
    pub(crate) store: &'a AppStore,
    pub(crate) credentials: &'a JsonCredentialStore,
    pub(crate) provider_id: &'a str,
    pub(crate) base_url: &'a str,
    pub(crate) reasoning_effort: Option<&'a str>,
    pub(crate) run_id: &'a str,
    pub(crate) signal: &'a CancellationSignal,
    pub(crate) sequence: &'a AtomicU64,
}

pub(crate) async fn generate_stage(
    context: &GenerationStageContext<'_>,
    input_hash: Option<String>,
    request: &GenerationRequest,
) -> Result<GenerationResponse, AiError> {
    let shared_binding = legacy_shared_binding(context.store, context.run_id)
        .await
        .filter(|_| matches!(context.provider_id, "openai" | "deepseek"));
    if shared_binding.is_none()
        && matches!(context.provider_id, "openai" | "deepseek")
        && legacy_run_requires_shared_binding(context.store, context.run_id).await?
    {
        return Err(AiError::TemporaryUnavailable);
    }
    // Legacy typed tasks use the shared Codex-derived retry policy while their
    // candidate projection is being migrated. The adapter only adds the
    // provider's explicit retry-after floor; it does not own another policy.
    const LEGACY_RETRY_POLICY: RetryPolicy = RetryPolicy {
        max_attempts: 2,
        base_delay_ms: 250,
        max_delay_ms: 2_000,
    };
    for attempt_index in 0..LEGACY_RETRY_POLICY.max_attempts {
        let started = Instant::now();
        let stage_id = begin_stage(
            context.store,
            context.run_id,
            "generating",
            input_hash.clone(),
        )
        .await?;
        let response = if let Some(binding) = shared_binding.clone() {
            generate_shared_text(
                context.app,
                context.store,
                context.provider_id,
                context.reasoning_effort,
                request,
                context.signal,
                binding,
            )
            .await
        } else {
            generate_once(
                context.provider_id,
                context.base_url,
                context.credentials,
                Some(context.app.state::<AgentModelResolver>().inner().clone()),
                context.reasoning_effort,
                request,
                context.signal,
            )
            .await
        };
        match response {
            Ok(response) => {
                complete_stage(
                    context.store,
                    &stage_id,
                    Some(hash_text(&response.content)),
                    Some(&response.usage),
                    started,
                )
                .await?;
                emit_progress(context.app, context.run_id, context.sequence);
                return Ok(response);
            }
            Err(error) => {
                fail_stage(context.store, &stage_id, &error, started).await?;
                emit_progress(context.app, context.run_id, context.sequence);
                if shared_binding.is_none()
                    && let Some(delay) =
                        legacy_retry_delay(&LEGACY_RETRY_POLICY, &error, attempt_index)
                {
                    context
                        .signal
                        .race(tokio::time::sleep(delay))
                        .await
                        .map_err(|_| AiError::Canceled)?;
                    continue;
                }
                return Err(provider_error(error));
            }
        }
    }
    Err(AiError::TemporaryUnavailable)
}

async fn legacy_shared_binding(
    store: &AppStore,
    run_id: &str,
) -> Option<crate::agent::repository::legacy_ai_runs::Binding> {
    let id = uuid::Uuid::parse_str(run_id).ok()?;
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let connection = store.connection().ok()?;
        crate::agent::repository::legacy_ai_runs::read(&connection, id)
            .ok()
            .flatten()
    })
    .await
    .ok()
    .flatten()
}

async fn legacy_run_requires_shared_binding(
    store: &AppStore,
    run_id: &str,
) -> Result<bool, AiError> {
    let id = uuid::Uuid::parse_str(run_id).map_err(|_| AiError::TemporaryUnavailable)?;
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let connection = store.connection()?;
        let task_kind = connection
            .query_row(
                "select task_kind from ai_runs where id = ?1",
                [id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(task_kind
            .as_deref()
            .is_some_and(super::run_repository::uses_shared_agent_compatibility))
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)?
}

async fn generate_shared_text(
    app: &AppHandle,
    store: &AppStore,
    provider_id: &str,
    reasoning_effort: Option<&str>,
    request: &GenerationRequest,
    signal: &CancellationSignal,
    binding: crate::agent::repository::legacy_ai_runs::Binding,
) -> Result<GenerationResponse, ProviderError> {
    debug_assert!(matches!(provider_id, "openai" | "deepseek"));
    let models = app.state::<AgentModelResolver>().inner().clone();
    let prepared = models
        .prepare(
            AgentHost::Review,
            Some(AgentModelSelection {
                provider_id: provider_id.to_owned(),
                model_id: request.model_id.clone(),
                reasoning_effort: reasoning_effort.map(str::to_owned),
                service_tier: None,
            }),
        )
        .await
        .map_err(|_| ProviderError::new(ProviderErrorKind::InvalidRequest))?;
    let sessions = app
        .try_state::<Arc<AgentSessionService>>()
        .ok_or_else(|| ProviderError::new(ProviderErrorKind::TemporaryUnavailable))?;
    let response = super::paper_reading_shared_transport::call_prepared_text_submission(
        store.clone(),
        sessions.inner().clone(),
        prepared,
        super::shared_responses::provider_input(request),
        *binding.conversation_id.as_uuid(),
        binding.request_id,
        signal.clone(),
    )
    .await
    .map_err(super::shared_responses::provider_error)?;
    Ok(GenerationResponse {
        model_id: request.model_id.clone(),
        content: response.content,
        usage: ProviderUsage {
            input_tokens: response
                .input_tokens
                .and_then(|value| u64::try_from(value).ok()),
            output_tokens: response
                .output_tokens
                .and_then(|value| u64::try_from(value).ok()),
            cache_hit_tokens: None,
            cache_miss_tokens: None,
        },
    })
}

async fn generate_once(
    provider_id: &str,
    _base_url: &str,
    _credentials: &JsonCredentialStore,
    models: Option<AgentModelResolver>,
    reasoning_effort: Option<&str>,
    request: &GenerationRequest,
    signal: &CancellationSignal,
) -> Result<GenerationResponse, ProviderError> {
    match provider_id {
        "fixture-generation" => {
            FixtureGenerationProvider::default()
                .generate(request, signal)
                .await
        }
        "deepseek" => {
            let models =
                models.ok_or_else(|| ProviderError::new(ProviderErrorKind::InvalidRequest))?;
            let prepared = models
                .prepare(
                    super::agent_configuration::AgentHost::Review,
                    Some(super::agent_configuration::AgentModelSelection {
                        provider_id: "deepseek".to_owned(),
                        model_id: request.model_id.clone(),
                        reasoning_effort: reasoning_effort.map(str::to_owned),
                        service_tier: None,
                    }),
                )
                .await
                .map_err(|_| ProviderError::new(ProviderErrorKind::InvalidRequest))?;
            let response = tokio::time::timeout(
                request.timeout,
                super::paper_reading_shared_transport::call_prepared_text(
                    prepared,
                    super::shared_responses::provider_input(request),
                    true,
                    request.max_output_tokens,
                    Some(signal.clone()),
                ),
            )
            .await
            .map_err(|_| ProviderError::new(ProviderErrorKind::Timeout))?
            .map_err(super::shared_responses::provider_error)?;
            Ok(GenerationResponse {
                model_id: request.model_id.clone(),
                content: response.content,
                usage: ProviderUsage {
                    input_tokens: response
                        .input_tokens
                        .and_then(|value| u64::try_from(value).ok()),
                    output_tokens: response
                        .output_tokens
                        .and_then(|value| u64::try_from(value).ok()),
                    cache_hit_tokens: None,
                    cache_miss_tokens: None,
                },
            })
        }
        "openai" => {
            let models =
                models.ok_or_else(|| ProviderError::new(ProviderErrorKind::InvalidRequest))?;
            SharedResponsesGenerationProvider::new(models, reasoning_effort.map(str::to_owned))
                .generate(request, signal)
                .await
        }
        _ => Err(ProviderError::new(
            super::provider::ProviderErrorKind::InvalidRequest,
        )),
    }
}

pub(crate) fn validate_diagnostic(content: &str) -> Result<(), AiError> {
    let value: serde_json::Value = serde_json::from_str(content)
        .map_err(|_| AiError::InvalidInput("provider 返回的结构化结果无效".to_owned()))?;
    if value.get("kind").and_then(serde_json::Value::as_str) == Some("provider_diagnostic_v1")
        && value.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
    {
        Ok(())
    } else {
        Err(AiError::InvalidInput(
            "provider 诊断结果不符合 schema".to_owned(),
        ))
    }
}

fn provider_error(error: ProviderError) -> AiError {
    if let Some(cause) = error.model_error {
        return super::paper_reading_shared_transport::map_model_error(cause);
    }
    match error.kind {
        super::provider::ProviderErrorKind::Canceled => AiError::Canceled,
        super::provider::ProviderErrorKind::Timeout => AiError::Timeout,
        kind => AiError::Provider(kind.code()),
    }
}
pub(crate) fn hash_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}
fn legacy_retry_delay(
    policy: &RetryPolicy,
    error: &ProviderError,
    completed_attempts: u8,
) -> Option<Duration> {
    let model_error = match error.kind {
        ProviderErrorKind::RateLimited => ModelError::RateLimited,
        ProviderErrorKind::TemporaryUnavailable | ProviderErrorKind::Timeout => {
            ModelError::Unavailable
        }
        _ => ModelError::InvalidRequest,
    };
    let RetryDecision::Retry { delay_ms } = policy.decide(model_error, completed_attempts + 1)
    else {
        return None;
    };
    let retry_after = error.retry_after_ms.unwrap_or(0).min(policy.max_delay_ms);
    Some(Duration::from_millis(delay_ms.max(retry_after)))
}
pub(crate) fn usage_tuple(usage: &ProviderUsage) -> Result<(i64, i64, i64, i64), AiError> {
    Ok((
        to_i64(usage.input_tokens)?,
        to_i64(usage.output_tokens)?,
        to_i64(usage.cache_hit_tokens)?,
        to_i64(usage.cache_miss_tokens)?,
    ))
}
fn to_i64(value: Option<u64>) -> Result<i64, AiError> {
    value.map_or(Ok(0), |item| {
        i64::try_from(item)
            .map_err(|_| AiError::InvalidInput("provider 用量超出支持范围".to_owned()))
    })
}

#[cfg(test)]
mod tests {
    use super::legacy_retry_delay;
    use crate::ai::provider::{ProviderError, ProviderErrorKind};
    use research_agent_core::resilience::RetryPolicy;
    use std::time::Duration;

    #[test]
    fn legacy_retry_uses_shared_policy_and_provider_retry_after_floor() {
        let policy = RetryPolicy {
            max_attempts: 2,
            base_delay_ms: 250,
            max_delay_ms: 2_000,
        };
        let error = ProviderError::with_retry_after(ProviderErrorKind::RateLimited, Some(900));
        assert_eq!(
            legacy_retry_delay(&policy, &error, 0),
            Some(Duration::from_millis(900))
        );
        assert_eq!(legacy_retry_delay(&policy, &error, 1), None);
        let permanent = ProviderError::new(ProviderErrorKind::InvalidRequest);
        assert_eq!(legacy_retry_delay(&policy, &permanent, 0), None);
    }
}
