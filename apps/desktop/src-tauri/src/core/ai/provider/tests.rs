use std::{collections::HashSet, time::Duration};

use research_literature_library_native::analysis_workflow::candidate::decode_candidate_content;

use super::*;

fn request(format: GenerationResponseFormat) -> GenerationRequest {
    GenerationRequest {
        model_id: "fixture-v1".to_owned(),
        messages: vec![
            GenerationMessage {
                role: GenerationRole::System,
                content: "Return JSON only.".to_owned(),
            },
            GenerationMessage {
                role: GenerationRole::User,
                content: "Synthetic provider fixture".to_owned(),
            },
        ],
        max_output_tokens: 100,
        thinking_mode: ThinkingMode::Disabled,
        response_format: format,
        timeout: Duration::from_secs(1),
    }
}

fn suggestion_request(kind: &str) -> GenerationRequest {
    GenerationRequest {
        model_id: "fixture-v1".to_owned(),
        messages: vec![GenerationMessage {
            role: GenerationRole::User,
            content: serde_json::json!({ "task": kind }).to_string(),
        }],
        max_output_tokens: 1_000,
        thinking_mode: ThinkingMode::Disabled,
        response_format: GenerationResponseFormat::JsonObject,
        timeout: Duration::from_secs(1),
    }
}

fn research_request(kind: &str) -> GenerationRequest {
    let evidence = [
        serde_json::json!({
            "citationKey": "ev-source-1",
            "quoteExact": "Doe (2024). Grounded systems."
        }),
        serde_json::json!({
            "citationKey": "ev-source-2",
            "quoteExact": "Roe (2025). Evidence systems."
        }),
    ];
    GenerationRequest {
        model_id: "fixture-v1".to_owned(),
        messages: vec![GenerationMessage {
            role: GenerationRole::User,
            content: serde_json::json!({
                "task": kind,
                "viewKind": "taxonomy",
                "title": "Grounded systems",
                "scopeNote": "Selected evidence only.",
                "evidence": evidence
            })
            .to_string(),
        }],
        max_output_tokens: 1_000,
        thinking_mode: ThinkingMode::Disabled,
        response_format: GenerationResponseFormat::JsonObject,
        timeout: Duration::from_secs(1),
    }
}

#[test]
fn fixture_provider_uses_the_same_typed_contract_without_credentials() {
    tauri::async_runtime::block_on(async {
        let provider = FixtureGenerationProvider::default();
        let capabilities = provider.capabilities();
        assert_eq!(capabilities.provider_id, "fixture-generation");
        assert!(!capabilities.is_cloud);
        let response = provider
            .generate(
                &request(GenerationResponseFormat::JsonObject),
                &CancellationSignal::default(),
            )
            .await
            .expect("fixture generation");
        assert_eq!(response.model_id, "fixture-v1");
        assert_eq!(response.usage.input_tokens, Some(12));
        serde_json::from_str::<serde_json::Value>(&response.content)
            .expect("fixture returns valid JSON");
    });
}

#[test]
fn qwen_generation_capabilities_include_vision_streaming() {
    let capabilities = capabilities_for("qwen").expect("qwen capabilities");
    assert!(capabilities.supports_vision_input);
    assert!(capabilities.supports_streaming);
    assert!(capabilities.supports_cancellation);
}

#[test]
fn fixture_provider_returns_all_frozen_suggestion_contracts() {
    tauri::async_runtime::block_on(async {
        let provider = FixtureGenerationProvider::default();
        let allowed = HashSet::from(["ev-page-1".to_owned()]);
        for kind in [
            "concept_suggestion_v1",
            "definition_suggestion_v1",
            "relation_suggestion_v1",
            "attention_suggestion_v1",
        ] {
            let response = provider
                .generate(&suggestion_request(kind), &CancellationSignal::default())
                .await
                .expect("fixture suggestion generation");
            let payload = decode_candidate_content(&response.content, &allowed)
                .expect("fixture suggestion follows frozen schema");
            assert_eq!(payload.kind(), kind);
        }
    });
}

#[test]
fn fixture_provider_returns_both_frozen_research_candidate_contracts() {
    tauri::async_runtime::block_on(async {
        let provider = FixtureGenerationProvider::default();
        let allowed = HashSet::from(["ev-source-1".to_owned(), "ev-source-2".to_owned()]);
        for kind in ["reference_record_candidate_v1", "derived_research_view_v1"] {
            let response = provider
                .generate(&research_request(kind), &CancellationSignal::default())
                .await
                .expect("fixture research generation");
            let payload = decode_candidate_content(&response.content, &allowed)
                .expect("fixture research candidate follows frozen schema");
            assert_eq!(payload.kind(), kind);
        }
    });
}

#[test]
fn cancellation_stops_a_delayed_fixture_without_persisting_a_response() {
    tauri::async_runtime::block_on(async {
        let provider = FixtureGenerationProvider::with_behavior(FixtureBehavior::Delayed {
            content: "late response".to_owned(),
            delay: Duration::from_secs(30),
        });
        let cancellation = CancellationSignal::default();
        let cancel_task = cancellation.clone();
        tauri::async_runtime::spawn(async move {
            tokio::task::yield_now().await;
            cancel_task.cancel();
        });
        let error = provider
            .generate(
                &request(GenerationResponseFormat::JsonObject),
                &cancellation,
            )
            .await
            .expect_err("canceled fixture");
        assert_eq!(error.kind, ProviderErrorKind::Canceled);
    });
}

#[test]
fn retryability_is_explicit_and_bounded_to_transient_categories() {
    for retryable in [
        ProviderErrorKind::RateLimited,
        ProviderErrorKind::TemporaryUnavailable,
        ProviderErrorKind::Timeout,
    ] {
        assert!(retryable.is_retryable());
    }
    for permanent in [
        ProviderErrorKind::Authentication,
        ProviderErrorKind::InsufficientBalance,
        ProviderErrorKind::InvalidRequest,
        ProviderErrorKind::InvalidResponse,
        ProviderErrorKind::ForbiddenToolCall,
    ] {
        assert!(!permanent.is_retryable());
    }

    let provider = FixtureGenerationProvider::with_behavior(FixtureBehavior::Failure(
        ProviderErrorKind::RateLimited,
    ));
    let error = tauri::async_runtime::block_on(provider.generate(
        &request(GenerationResponseFormat::JsonObject),
        &CancellationSignal::default(),
    ))
    .expect_err("fixture failure");
    assert_eq!(error.kind.code(), "rate_limited");
}
