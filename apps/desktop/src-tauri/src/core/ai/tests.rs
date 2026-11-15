use super::candidate_test_fixture;
use std::{fs, path::PathBuf};

use rusqlite::Connection;
use uuid::Uuid;

use crate::storage::migrate;

use super::{
    credential::JsonCredentialStore,
    error::AiError,
    llm_configuration,
    model::{AiProviderSettingsDraft, AiRunProgressEvent, AiTaskSpecDraft},
    run_repository,
    run_state_repository::{self, StageCompletion},
    settings_repository,
};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("research-assistant-ai-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create isolated AI test directory");
        Self(path)
    }

    fn credentials(&self) -> JsonCredentialStore {
        JsonCredentialStore::from_path(self.0.join("credentials.json"))
    }
}

#[test]
fn model_routes_prefer_run_then_use_case_then_kind_default() {
    let mut connection = database();
    let directory = TestDirectory::new();
    let credentials = directory.credentials();
    let configuration = llm_configuration::get_configuration(&connection, &credentials)
        .expect("load OpenAI provider");
    let openai = configuration
        .providers
        .iter()
        .find(|provider| provider.provider_id == "openai")
        .expect("OpenAI provider");
    assert_eq!(openai.auth_mode, "chatgpt-login");
    assert_eq!(openai.protocol, "codex-cli");

    llm_configuration::save_routes(
        &mut connection,
        &credentials,
        vec![
            super::model::LlmModelRouteDraft {
                route_kind: "default".to_owned(),
                route_id: "text".to_owned(),
                selection: Some(super::model::LlmModelSelectionDraft {
                    provider_id: "deepseek".to_owned(),
                    model_id: "deepseek-chat".to_owned(),
                    reasoning_effort: None,
                }),
            },
            super::model::LlmModelRouteDraft {
                route_kind: "use_case".to_owned(),
                route_id: "paper_overview".to_owned(),
                selection: Some(super::model::LlmModelSelectionDraft {
                    provider_id: "qwen".to_owned(),
                    model_id: "qwen-vl".to_owned(),
                    reasoning_effort: None,
                }),
            },
        ],
    )
    .expect("save route precedence");
    let selected = llm_configuration::resolve_model_selection(&connection, "paper_overview", None)
        .expect("use-case route");
    assert_eq!(
        (
            selected.provider_id.as_str(),
            selected.model_id.as_str(),
            selected.reasoning_effort.as_deref()
        ),
        ("qwen", "qwen-vl", None)
    );
    let fallback =
        llm_configuration::resolve_model_selection(&connection, "selection_explanation", None)
            .expect("text fallback");
    assert_eq!(fallback.model_id, "deepseek-chat");
    let override_selection = super::model::LlmModelSelectionDraft {
        provider_id: "openai".to_owned(),
        model_id: "gpt-5.3-codex".to_owned(),
        reasoning_effort: Some("xhigh".to_owned()),
    };
    let overridden = llm_configuration::resolve_model_selection(
        &connection,
        "paper_overview",
        Some(&override_selection),
    )
    .expect("run override");
    assert_eq!(overridden.model_id, "gpt-5.3-codex");

    connection
        .execute(
            "insert into ai_runs
         (id, task_kind, status, provider_id, model_id, reasoning_effort, pipeline_version,
          prompt_version, output_schema_version, user_goal, scope_hash, input_item_count,
          max_input_tokens, max_output_tokens, timeout_ms, cancel_requested, created_at, updated_at)
         values ('openai-run', 'provider_diagnostic', 'draft', 'openai', 'gpt-5.4', 'high',
                 'v1', 'v1', 1, '', ?1, 0, 128, 64, 5000, 0, 'now', 'now')",
            ["a".repeat(64)],
        )
        .expect("OpenAI run satisfies legacy foreign key");
}

#[test]
fn llm_configuration_uses_json_defaults_and_persists_model_routes() {
    let mut connection = database();
    let directory = TestDirectory::new();
    let credentials = directory.credentials();
    let initial = llm_configuration::get_configuration(&connection, &credentials)
        .expect("load provider catalog");
    let deepseek = initial
        .providers
        .iter()
        .find(|provider| provider.provider_id == "deepseek")
        .expect("deepseek config");
    assert!(deepseek.uses_default_base_url);
    assert_eq!(deepseek.base_url, "https://api.deepseek.com");
    let qwen = initial
        .providers
        .iter()
        .find(|provider| provider.provider_id == "qwen")
        .expect("qwen config");
    assert!(qwen.model_kinds.iter().any(|kind| kind == "vision"));
    let chatgpt_codex = initial
        .providers
        .iter()
        .find(|provider| provider.provider_id == "openai")
        .expect("ChatGPT Codex config");
    assert!(
        chatgpt_codex
            .model_kinds
            .iter()
            .any(|kind| kind == "vision")
    );
    assert!(
        !initial
            .providers
            .iter()
            .any(|provider| provider.provider_id == "qwen-embedding")
    );

    let saved = llm_configuration::save_routes(
        &mut connection,
        &credentials,
        vec![super::model::LlmModelRouteDraft {
            route_kind: "default".to_owned(),
            route_id: "vision".to_owned(),
            selection: Some(super::model::LlmModelSelectionDraft {
                provider_id: "openai".to_owned(),
                model_id: "gpt-5.6-sol".to_owned(),
                reasoning_effort: None,
            }),
        }],
    )
    .expect("save model route");
    assert_eq!(
        saved
            .default_models
            .iter()
            .find(|route| route.route_id == "vision")
            .and_then(|route| route.selection.as_ref())
            .map(|selection| selection.model_id.as_str()),
        Some("gpt-5.6-sol")
    );
    let inherited_vision =
        llm_configuration::resolve_model_selection(&connection, "paper_overview", None)
            .expect("paper overview inherits default vision route");
    assert_eq!(
        (
            inherited_vision.provider_id.as_str(),
            inherited_vision.model_id.as_str()
        ),
        ("openai", "gpt-5.6-sol")
    );
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("open isolated database");
    migrate(&mut connection).expect("apply migrations");
    connection
}

fn diagnostic_draft(provider_id: &str) -> AiTaskSpecDraft {
    AiTaskSpecDraft {
        task_kind: "provider_diagnostic".to_owned(),
        provider_id: provider_id.to_owned(),
        model_selection: None,
        user_goal: String::new(),
        pipeline_version: "diagnostic-v1".to_owned(),
        prompt_version: "diagnostic-v1".to_owned(),
        output_schema_version: 1,
        max_input_tokens: 128,
        max_output_tokens: 64,
        timeout_ms: 5_000,
        scope_items: Vec::new(),
        overview_fields: Vec::new(),
        overview_profile_selection_revision: None,
        paper_template_ref: None,
        suggestion_candidate_kind: None,
        matrix_synthesis_context: None,
        base_candidate_id: None,
        base_candidate_revision: None,
        confirmed_scope_hash: None,
        cloud_consent_version: None,
    }
}

#[test]
fn provider_settings_require_credentials_and_allowlisted_urls() {
    let connection = database();
    let directory = TestDirectory::new();
    let credentials = directory.credentials();
    let draft = AiProviderSettingsDraft {
        provider_id: "deepseek".to_owned(),
        enabled: true,
        base_url: "https://api.deepseek.com".to_owned(),
        region: String::new(),
        model_id: "deepseek-v4-flash".to_owned(),
        thinking_mode: "disabled".to_owned(),
        timeout_ms: 60_000,
    };
    assert!(matches!(
        settings_repository::save_provider_settings(&connection, &credentials, draft.clone()),
        Err(AiError::CredentialMissing)
    ));
    let secret = format!("test-{}", Uuid::new_v4());
    credentials
        .set("deepseek", &secret)
        .expect("set fixture credential");
    let saved = settings_repository::save_provider_settings(&connection, &credentials, draft)
        .expect("save enabled DeepSeek settings");
    assert!(saved.enabled);
    assert!(
        saved
            .credential_mask
            .as_deref()
            .is_some_and(|mask| !mask.contains(&secret))
    );
    let persisted: String = connection.query_row(
        "select provider_id || base_url || model_id from ai_provider_settings where provider_id = 'deepseek'",
        [], |row| row.get(0),
    ).expect("read non-secret settings");
    assert!(!persisted.contains(&secret));

    let unsafe_url = AiProviderSettingsDraft {
        base_url: "https://example.com".to_owned(),
        ..saved_draft(&saved)
    };
    assert!(matches!(
        settings_repository::save_provider_settings(&connection, &credentials, unsafe_url),
        Err(AiError::InvalidInput(_))
    ));

    let retired_model = AiProviderSettingsDraft {
        model_id: "deepseek-chat".to_owned(),
        ..saved_draft(&saved)
    };
    assert!(matches!(
        settings_repository::save_provider_settings(&connection, &credentials, retired_model),
        Err(AiError::InvalidInput(_))
    ));
}

fn saved_draft(settings: &super::model::AiProviderSettings) -> AiProviderSettingsDraft {
    AiProviderSettingsDraft {
        provider_id: settings.provider_id.clone(),
        enabled: settings.enabled,
        base_url: settings.base_url.clone(),
        region: settings.region.clone(),
        model_id: settings.model_id.clone(),
        thinking_mode: settings.thinking_mode.clone(),
        timeout_ms: settings.timeout_ms,
    }
}

#[test]
fn scope_preview_is_stable_and_start_requires_exact_confirmation() {
    let mut connection = database();
    let directory = TestDirectory::new();
    let credentials = directory.credentials();
    let mut draft = diagnostic_draft("fixture-generation");
    let first = candidate_test_fixture::preview(&connection, &credentials, &draft)
        .expect("preview diagnostic");
    let second =
        candidate_test_fixture::preview(&connection, &credentials, &draft).expect("repeat preview");
    assert_eq!(first.scope_hash, second.scope_hash);
    assert!(matches!(
        run_repository::insert_run(&mut connection, &credentials, &draft),
        Err(AiError::InvalidInput(_))
    ));
    draft.confirmed_scope_hash = Some(first.scope_hash);
    let run = run_repository::insert_run(&mut connection, &credentials, &draft)
        .expect("insert confirmed run");
    assert_eq!(run.status, "queued");
    assert!(run.scope_items.is_empty());
}

#[test]
fn stages_and_cancel_reach_explicit_terminal_states_transactionally() {
    let mut connection = database();
    let directory = TestDirectory::new();
    let credentials = directory.credentials();
    let mut draft = diagnostic_draft("fixture-generation");
    draft.confirmed_scope_hash = Some(
        candidate_test_fixture::preview(&connection, &credentials, &draft)
            .expect("preview")
            .scope_hash,
    );
    let run = run_repository::insert_run(&mut connection, &credentials, &draft).expect("run");
    let stage_id = run_state_repository::begin_stage(
        &mut connection,
        &run.id,
        "resolving_scope",
        Some(&run.scope_hash),
    )
    .expect("begin stage");
    run_state_repository::complete_stage(
        &connection,
        &stage_id,
        StageCompletion {
            output_hash: Some(&run.scope_hash),
            input_tokens: None,
            output_tokens: None,
            duration_ms: 1,
        },
    )
    .expect("complete stage");
    let failed_attempt = run_state_repository::begin_stage(
        &mut connection,
        &run.id,
        "generating",
        Some(&run.scope_hash),
    )
    .expect("begin first generation attempt");
    run_state_repository::fail_stage(
        &connection,
        &failed_attempt,
        "rate_limited",
        "provider attempt 失败",
        2,
    )
    .expect("record failed attempt");
    let retry_attempt = run_state_repository::begin_stage(
        &mut connection,
        &run.id,
        "generating",
        Some(&run.scope_hash),
    )
    .expect("begin retry attempt");
    run_state_repository::complete_stage(
        &connection,
        &retry_attempt,
        StageCompletion {
            output_hash: Some(&run.scope_hash),
            input_tokens: Some(2),
            output_tokens: Some(1),
            duration_ms: 1,
        },
    )
    .expect("complete retry attempt");
    assert!(run_state_repository::request_cancel(&connection, &run.id).expect("request cancel"));
    assert!(matches!(
        run_state_repository::begin_stage(&mut connection, &run.id, "retrieving", None),
        Err(AiError::Canceled)
    ));
    run_state_repository::finish_run(
        &mut connection,
        &run.id,
        "canceled",
        Some("canceled"),
        Some("AI 运行已取消"),
        None,
    )
    .expect("finish canceled");
    let canceled = run_repository::get_run(&connection, &run.id).expect("load canceled run");
    assert_eq!(canceled.status, "canceled");
    assert!(canceled.cancel_requested);
    let generation_attempts = canceled
        .stages
        .iter()
        .filter(|stage| stage.stage_kind == "generating")
        .collect::<Vec<_>>();
    assert_eq!(generation_attempts.len(), 2);
    assert_eq!(
        (
            generation_attempts[0].attempt,
            generation_attempts[0].status.as_str()
        ),
        (1, "failed")
    );
    assert_eq!(
        (
            generation_attempts[1].attempt,
            generation_attempts[1].status.as_str()
        ),
        (2, "succeeded")
    );
}

#[test]
fn cloud_run_requires_current_consent_after_preview() {
    let mut connection = database();
    let directory = TestDirectory::new();
    let credentials = directory.credentials();
    credentials
        .set("deepseek", &format!("test-{}", Uuid::new_v4()))
        .expect("credential");
    settings_repository::save_provider_settings(
        &connection,
        &credentials,
        AiProviderSettingsDraft {
            provider_id: "deepseek".to_owned(),
            enabled: true,
            base_url: "https://api.deepseek.com".to_owned(),
            region: String::new(),
            model_id: "deepseek-v4-flash".to_owned(),
            thinking_mode: "disabled".to_owned(),
            timeout_ms: 60_000,
        },
    )
    .expect("enable DeepSeek");
    let mut draft = diagnostic_draft("deepseek");
    let preview =
        candidate_test_fixture::preview(&connection, &credentials, &draft).expect("preview cloud");
    draft.confirmed_scope_hash = Some(preview.scope_hash);
    assert!(matches!(
        run_repository::insert_run(&mut connection, &credentials, &draft),
        Err(AiError::InvalidInput(_))
    ));
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".to_owned());
    let run =
        run_repository::insert_run(&mut connection, &credentials, &draft).expect("consented run");
    assert!(run.cloud_consented_at.is_some());
}

#[test]
fn fixture_pipeline_records_every_stage_and_normalized_usage() {
    let mut connection = database();
    let directory = TestDirectory::new();
    let credentials = directory.credentials();
    let mut draft = diagnostic_draft("fixture-generation");
    draft.confirmed_scope_hash = Some(
        candidate_test_fixture::preview(&connection, &credentials, &draft)
            .expect("preview fixture pipeline")
            .scope_hash,
    );
    let run = run_repository::insert_run(&mut connection, &credentials, &draft).expect("run");
    let mut input_hash = run.scope_hash.clone();
    for stage_kind in [
        "resolving_scope",
        "retrieving",
        "extracting_evidence",
        "generating",
        "validating",
    ] {
        let stage_id = run_state_repository::begin_stage(
            &mut connection,
            &run.id,
            stage_kind,
            Some(&input_hash),
        )
        .expect("begin pipeline stage");
        input_hash = format!("{:0<64}", stage_kind);
        run_state_repository::complete_stage(
            &connection,
            &stage_id,
            StageCompletion {
                output_hash: Some(&input_hash),
                input_tokens: Some(12),
                output_tokens: Some(8),
                duration_ms: 1,
            },
        )
        .expect("complete pipeline stage");
    }
    run_state_repository::finish_run(
        &mut connection,
        &run.id,
        "partial",
        Some("candidate_pipeline_pending"),
        Some("业务候选管线尚未接入"),
        Some((12, 8, 0, 12)),
    )
    .expect("finish fixture pipeline");
    let completed = run_repository::get_run(&connection, &run.id).expect("load pipeline");
    assert_eq!(completed.stages.len(), 5);
    assert!(
        completed
            .stages
            .iter()
            .all(|stage| stage.status == "succeeded")
    );
    assert_eq!(
        (completed.input_tokens, completed.output_tokens),
        (Some(12), Some(8))
    );
}

#[test]
fn runtime_secret_is_confined_to_the_credential_file() {
    let directory = TestDirectory::new();
    let database_path = directory.0.join("runtime.sqlite3");
    let mut connection = Connection::open(&database_path).expect("open file database");
    migrate(&mut connection).expect("migrate file database");
    let credentials = directory.credentials();
    let secret = format!("secret-{}", Uuid::new_v4());
    credentials
        .set("deepseek", &secret)
        .expect("set runtime secret");
    settings_repository::save_provider_settings(
        &connection,
        &credentials,
        AiProviderSettingsDraft {
            provider_id: "deepseek".to_owned(),
            enabled: true,
            base_url: "https://api.deepseek.com".to_owned(),
            region: String::new(),
            model_id: "deepseek-v4-flash".to_owned(),
            thinking_mode: "disabled".to_owned(),
            timeout_ms: 60_000,
        },
    )
    .expect("save non-secret provider settings");
    let mut draft = diagnostic_draft("deepseek");
    let preview =
        candidate_test_fixture::preview(&connection, &credentials, &draft).expect("preview");
    draft.confirmed_scope_hash = Some(preview.scope_hash);
    draft.cloud_consent_version = Some("ai-cloud-consent-v1".to_owned());
    run_repository::insert_run(&mut connection, &credentials, &draft).expect("persist run");
    drop(connection);

    for entry in fs::read_dir(&directory.0).expect("read test artifacts") {
        let path = entry.expect("artifact entry").path();
        if path.file_name().and_then(|name| name.to_str()) == Some("credentials.json") {
            continue;
        }
        let bytes = fs::read(&path).expect("read non-credential artifact");
        assert!(
            !bytes
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
        );
    }
}

#[test]
fn progress_event_uses_monotonic_sequence_ipc_shape() {
    let value = serde_json::to_value(AiRunProgressEvent {
        run_id: "run-1".to_owned(),
        sequence: 3,
    })
    .expect("serialize progress event");
    assert_eq!(
        value,
        serde_json::json!({ "runId": "run-1", "sequence": 3 })
    );
}
