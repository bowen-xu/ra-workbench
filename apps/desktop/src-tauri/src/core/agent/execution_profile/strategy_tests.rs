//! Policy identity is independent of the transport's protocol capabilities.
use super::*;
use research_agent_core::{context::compaction_strategy::*, model::*};

struct Strategy(CompactionBinding);
impl CompactionStrategy for Strategy {
    fn binding(&self) -> CompactionBinding {
        self.0.clone()
    }
    fn validate_output(&self, _: &CompactionOutput) -> Result<(), CompactionFailure> {
        panic!("selection must not validate a candidate")
    }
    fn compact(&self, _: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        panic!("selection must not start compaction")
    }
}
struct Transport(Option<Arc<dyn CompactionStrategy>>);
impl ModelTransport for Transport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn prepared_compaction_strategy(&self) -> Option<Arc<dyn CompactionStrategy>> {
        self.0.clone()
    }
    fn supports_compaction_kind(&self, _: CompactionKind) -> bool {
        true
    }
    fn start(&self, _: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        panic!("selection must not start sampling")
    }
}

#[tokio::test]
async fn reset_opt_in_overrides_summary_policy_without_inference_or_remote_compatibility() {
    for (provider, model, mode) in [
        (
            "deepseek",
            "deepseek-v4-pro",
            AgentExecutionMode::DeepSeekNonThinking,
        ),
        (
            "openai",
            "gpt-5.6-terra",
            AgentExecutionMode::ChatGptDefault,
        ),
    ] {
        let model = ModelIdentity {
            provider_id: provider.into(),
            model_id: model.into(),
            revision: Some("frozen".into()),
        };
        let default = execution_profile(&model, mode).unwrap();
        assert_ne!(
            default.compaction_policy,
            NativeCompactionPolicy::TokenBudgetReset
        );
        let mut other = model.clone();
        other.revision = Some("changed".into());
        assert!(default.clone().with_token_budget_reset(&other).is_err());
        let profile = default.with_token_budget_reset(&model).unwrap();
        let strategy = profile
            .compaction_strategy(Arc::new(Transport(None)))
            .unwrap();
        let binding = strategy.binding();
        assert_eq!(
            binding,
            NativeCompactionPolicy::TokenBudgetReset.binding(model.clone())
        );
        let mut stored = binding.clone();
        stored.model = other;
        assert!(
            !NativeCompactionPolicy::TokenBudgetReset.accepts_remote_binding(&binding, &stored)
        );
        assert!(!strategy.accepts_binding(&stored));
        let candidate = prepare_compaction(
            strategy.as_ref(),
            CompactionRequest {
                operation_id: uuid::Uuid::new_v4(),
                conversation_id: uuid::Uuid::new_v4(),
                turn_id: uuid::Uuid::new_v4(),
                history_revision: 0,
                phase: CompactionPhase::Manual,
                binding,
                source: ModelRequest {
                    captured_history: None,
                    request_id: uuid::Uuid::new_v4(),
                    model,
                    messages: vec![],
                    tools: vec![],
                    max_output_tokens: 1,
                    parallel_tool_calls: false,
                    reasoning_effort: None,
                },
                max_result_bytes: 1000,
            },
        )
        .await
        .unwrap();
        assert!(matches!(candidate.output, CompactionOutput::ResetWindow));
        assert!(candidate.response_id.is_none());
        assert_eq!(candidate.usage, ModelUsage::default());
    }
}

#[test]
fn frozen_policy_rejects_another_strategy_even_when_transport_supports_it() {
    for (provider, model, mode) in [
        (
            "deepseek",
            "deepseek-v4-pro",
            AgentExecutionMode::DeepSeekNonThinking,
        ),
        (
            "openai",
            "gpt-5.6-terra",
            AgentExecutionMode::ChatGptDefault,
        ),
    ] {
        let profile = execution_profile(
            &ModelIdentity {
                provider_id: provider.into(),
                model_id: model.into(),
                revision: Some("frozen".into()),
            },
            mode,
        )
        .unwrap();
        let binding = profile.compaction_policy.binding(profile.model.clone());
        let expected: Arc<dyn CompactionStrategy> = Arc::new(Strategy(binding.clone()));
        let selected = profile
            .compaction_strategy(Arc::new(Transport(Some(expected.clone()))))
            .unwrap();
        assert_eq!(selected.binding(), binding);
        if profile.compaction_policy == NativeCompactionPolicy::Text {
            // Text I/O must be recomposed with the outer accounting/authority
            // transport, rather than retaining its raw inner strategy.
            assert!(!Arc::ptr_eq(&selected, &expected));
        } else {
            assert!(Arc::ptr_eq(&selected, &expected));
        }
        for change in 0..5 {
            let mut invalid = binding.clone();
            match change {
                0 => invalid.strategy_id = "another supplier policy".into(),
                1 => invalid.state_schema += 1,
                2 => invalid.kind = CompactionKind::RemoteWindow,
                3 => invalid.model.revision = Some("different".into()),
                _ => invalid.model.provider_id = "another provider".into(),
            }
            assert!(matches!(
                profile.compaction_strategy(Arc::new(Transport(Some(Arc::new(Strategy(invalid)))))),
                Err(AgentModelPreparationError::Transport(
                    ModelError::UnsupportedCapability
                ))
            ));
        }
    }
}

#[test]
fn missing_strategy_uses_only_the_explicit_text_policy() {
    for (provider, model, mode, text) in [
        (
            "deepseek",
            "deepseek-v4-flash",
            AgentExecutionMode::DeepSeekNonThinking,
            true,
        ),
        (
            "openai",
            "gpt-5.6-terra",
            AgentExecutionMode::ChatGptDefault,
            false,
        ),
    ] {
        let profile = execution_profile(
            &ModelIdentity {
                provider_id: provider.into(),
                model_id: model.into(),
                revision: None,
            },
            mode,
        )
        .unwrap();
        let result = profile.compaction_strategy(Arc::new(Transport(None)));
        assert_eq!(result.is_ok(), text);
        if let Ok(strategy) = result {
            assert_eq!(
                strategy.binding(),
                profile.compaction_policy.binding(profile.model)
            );
        }
    }
}

#[test]
fn explicit_full_window_policy_keeps_model_admission_and_does_not_change_defaults() {
    let mut model = ModelIdentity {
        provider_id: "openai".into(),
        model_id: "gpt-5.6-terra".into(),
        revision: None,
    };
    let mode = AgentExecutionMode::ChatGptDefault;
    let policy = NativeCompactionPolicy::ChatGptResponsesFullWindow;
    let profile = execution_profile_with_compaction(&model, mode, policy).unwrap();
    let binding = policy.binding(model.clone());
    assert_eq!(binding.kind, CompactionKind::RemoteWindow);
    assert_eq!(binding.strategy_id, "research.openai.responses-window.v1");
    let strategy: Arc<dyn CompactionStrategy> = Arc::new(Strategy(binding));
    let selected = profile
        .compaction_strategy(Arc::new(Transport(Some(strategy.clone()))))
        .unwrap();
    assert!(Arc::ptr_eq(&selected, &strategy));
    assert_eq!(
        execution_profile(&model, mode).unwrap().compaction_policy,
        NativeCompactionPolicy::ChatGptResponsesV2
    );
    let v2 = NativeCompactionPolicy::ChatGptResponsesV2.binding(model.clone());
    assert!(
        profile
            .compaction_strategy(Arc::new(Transport(Some(Arc::new(Strategy(v2))))))
            .is_err()
    );
    assert!(
        profile
            .compaction_strategy(Arc::new(Transport(None)))
            .is_err()
    );
    // This function supplies a protocol bound, not exact catalog admission.
    // Unsupported providers still cannot opt into Codex remote compaction.
    model.provider_id = "unreviewed".into();
    assert!(execution_profile_with_compaction(&model, mode, policy).is_err());
    model.provider_id = "deepseek".into();
    model.model_id = "deepseek-v4-pro".into();
    assert!(
        execution_profile_with_compaction(&model, AgentExecutionMode::DeepSeekNonThinking, policy)
            .is_err()
    );
}

#[test]
fn native_remote_read_opt_in_is_scoped_to_known_codex_codecs() {
    for policy in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        let model = ModelIdentity {
            provider_id: "openai".into(),
            model_id: "gpt-5.6-terra".into(),
            revision: Some("new".into()),
        };
        let target = policy.binding(model);
        let mut stored = target.clone();
        stored.model.revision = Some("old".into());
        let raw = Arc::new(Strategy(target.clone()));
        assert!(!raw.accepts_binding(&stored));
        let native = compaction::NativeReadStrategy { inner: raw, policy };
        assert!(native.accepts_binding(&stored));
        let mut other_model = stored.clone();
        other_model.model.model_id = "gpt-5.6-sol".into();
        assert!(native.accepts_binding(&other_model));
        for field in 0..4 {
            let mut incompatible = stored.clone();
            match field {
                0 => incompatible.model.provider_id = "custom".into(),
                1 => incompatible.model.model_id = String::new(),
                2 => incompatible.state_schema += 1,
                _ => incompatible.strategy_id = "foreign".into(),
            }
            assert!(!native.accepts_binding(&incompatible));
        }
    }
}

#[test]
fn native_read_validates_the_producer_codec_after_strategy_change() {
    let model = ModelIdentity {
        provider_id: "openai".into(),
        model_id: "gpt-5.6-terra".into(),
        revision: None,
    };
    let item = serde_json::json!({"type":"compaction","encrypted_content":"synthetic"});
    for (consumer, producer, output) in [
        (
            NativeCompactionPolicy::ChatGptResponsesV2,
            NativeCompactionPolicy::ChatGptResponsesFullWindow,
            CompactionOutput::RemoteWindow(vec![item.clone()]),
        ),
        (
            NativeCompactionPolicy::ChatGptResponsesFullWindow,
            NativeCompactionPolicy::ChatGptResponsesV2,
            CompactionOutput::RemoteOpaque(item.clone()),
        ),
    ] {
        let native = compaction::NativeReadStrategy {
            inner: Arc::new(Strategy(consumer.binding(model.clone()))),
            policy: consumer,
        };
        let binding = producer.binding(model.clone());
        assert!(native.accepts_binding(&binding));
        assert!(native.validate_stored_output(&binding, &output).is_ok());
        let invalid = match output {
            CompactionOutput::RemoteWindow(_) => CompactionOutput::RemoteWindow(vec![
                serde_json::json!({"type":"compaction","encrypted_content":42}),
            ]),
            _ => CompactionOutput::RemoteOpaque(
                serde_json::json!({"type":"compaction","encrypted_content":42}),
            ),
        };
        assert!(native.validate_stored_output(&binding, &invalid).is_err());
        let mut foreign = binding;
        foreign.model.provider_id = "other".into();
        assert!(native.validate_stored_output(&foreign, &output).is_err());
    }
}

#[test]
fn token_budget_settings_reject_invalid_config_and_exact_revision_drift() {
    use research_agent_core::context::token_budget::TokenBudgetSettings;
    let model = ModelIdentity {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        revision: Some("frozen".into()),
    };
    let profile = execution_profile(&model, AgentExecutionMode::DeepSeekNonThinking).unwrap();
    assert!(profile.token_budget_settings.is_none());
    assert!(
        profile
            .clone()
            .with_token_budget_settings(
                &model,
                TokenBudgetSettings {
                    reminder_threshold_tokens: Some(0),
                    ..Default::default()
                }
            )
            .is_err()
    );
    let mut changed = model.clone();
    changed.revision = None;
    assert!(
        profile
            .with_token_budget_settings(&changed, Default::default())
            .is_err()
    );
}
