use super::*;

#[test]
fn reasoning_summary_injection_requires_the_reviewed_exact_model() {
    use crate::agent::model::responses_reasoning::ResponsesReasoningSummary;
    let model = ModelIdentity {
        provider_id: "openai".into(),
        model_id: "gpt-5.6-terra".into(),
        revision: None,
    };
    let profile = execution_profile(&model, AgentExecutionMode::ChatGptDefault).unwrap();
    assert_eq!(profile.reasoning_summary, None);
    let selected = profile
        .clone()
        .with_responses_reasoning_summary(&model, Some(ResponsesReasoningSummary::Auto))
        .unwrap();
    assert_eq!(
        selected.reasoning_summary,
        Some(ResponsesReasoningSummary::Auto)
    );
    let mut revision = model.clone();
    revision.revision = Some("unknown-revision".into());
    assert!(
        profile
            .clone()
            .with_responses_reasoning_summary(&revision, Some(ResponsesReasoningSummary::Auto))
            .is_err()
    );
    assert!(
        execution_profile(&revision, AgentExecutionMode::ChatGptDefault)
            .unwrap()
            .with_responses_reasoning_summary(&revision, Some(ResponsesReasoningSummary::Auto))
            .is_err()
    );
    let deepseek = ModelIdentity {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        revision: None,
    };
    assert!(
        execution_profile(&deepseek, AgentExecutionMode::DeepSeekNonThinking)
            .unwrap()
            .with_responses_reasoning_summary(&deepseek, Some(ResponsesReasoningSummary::Auto))
            .is_err()
    );
    assert_eq!(
        selected
            .with_responses_reasoning_summary(&model, None)
            .unwrap()
            .reasoning_summary,
        None
    );
}

#[test]
fn compatibility_hash_is_trusted_exact_model_metadata() {
    let profile = execution_profile(&model(), AgentExecutionMode::DeepSeekNonThinking).unwrap();
    assert_eq!(profile.comp_hash, None);
    for invalid in ["", " ", "hash\n"] {
        assert!(
            profile
                .clone()
                .with_compaction_compatibility_hash(&model(), Some(invalid.into()))
                .is_err()
        );
    }
    let mut other = model();
    other.revision = Some("v2".into());
    assert!(
        profile
            .clone()
            .with_compaction_compatibility_hash(&other, Some("trusted".into()))
            .is_err()
    );
    let frozen = profile
        .with_compaction_compatibility_hash(&model(), Some("trusted".into()))
        .unwrap();
    assert_eq!(frozen.comp_hash.as_deref(), Some("trusted"));
}
use research_agent_core::{
    context::accounting::{ContextAccounting, Utf8JsonTokenEstimator},
    model::{ContentPart, ModelMessage},
};

#[test]
fn body_scope_is_frozen_without_clamping_its_explicit_trigger() {
    use research_agent_core::context::accounting::AutoCompactionScope;
    let profile = execution_profile(&model(), AgentExecutionMode::DeepSeekNonThinking)
        .unwrap()
        .with_context_budget_policy(
            &model(),
            NativeContextBudgetPolicy {
                scope: AutoCompactionScope::BodyAfterPrefix,
                effective_percent: 95,
                configured_trigger_tokens: Some(980),
            },
        )
        .unwrap();
    let budget = profile
        .context_budget_policy
        .resolve(Some(1000), 50)
        .unwrap()
        .unwrap();
    assert_eq!(
        profile.context_budget_policy.scope,
        AutoCompactionScope::BodyAfterPrefix
    );
    assert_eq!(
        budget
            .limits_for_scope(profile.context_budget_policy.scope)
            .unwrap(),
        (980, 950)
    );
}
fn model() -> ModelIdentity {
    ModelIdentity {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        revision: None,
    }
}
#[test]
fn injected_policy_changes_actual_trigger_and_preserves_hard_window() {
    let policy = NativeContextBudgetPolicy {
        scope: research_agent_core::context::accounting::AutoCompactionScope::Total,
        effective_percent: 80,
        configured_trigger_tokens: Some(60),
    };
    let profile = execution_profile(&model(), AgentExecutionMode::DeepSeekNonThinking)
        .unwrap()
        .with_context_budget_policy(&model(), policy)
        .unwrap();
    let budget = profile
        .context_budget_policy
        .resolve(Some(1000), 100)
        .unwrap()
        .unwrap();
    assert_eq!(budget.limits().unwrap(), (60, 800));
    let accounting = ContextAccounting {
        unknown_window_trigger_tokens: None,
        model: model(),
        budget: Some(budget),
        estimator: Arc::new(Utf8JsonTokenEstimator),
    };
    let small = vec![ModelMessage::User {
        content: vec![ContentPart::Text("short".into())],
        source_item_id: None,
    }];
    assert!(!accounting.should_compact(&model(), &small, &[]).unwrap());
    let body = vec![ModelMessage::User {
        content: vec![ContentPart::Text("a".repeat(400))],
        source_item_id: None,
    }];
    assert!(accounting.should_compact(&model(), &body, &[]).unwrap());
    assert!(accounting.check(&model(), &body, &[]).is_ok());
    let oversized = vec![ModelMessage::User {
        content: vec![ContentPart::Text("a".repeat(4000))],
        source_item_id: None,
    }];
    assert!(accounting.check(&model(), &oversized, &[]).is_err());
}
#[test]
fn default_retains_reviewed_formula_and_unknown_window_stays_unknown() {
    let default = NativeContextBudgetPolicy::REVIEWED_DEFAULT;
    assert_eq!(
        default
            .resolve(Some(10000), 100)
            .unwrap()
            .unwrap()
            .limits()
            .unwrap(),
        (9000, 9500)
    );
    let custom = NativeContextBudgetPolicy {
        scope: research_agent_core::context::accounting::AutoCompactionScope::Total,
        effective_percent: 75,
        configured_trigger_tokens: Some(500),
    };
    assert_eq!(custom.resolve(None, 100).unwrap(), None);
    assert_eq!(
        custom
            .resolve(Some(100), 100)
            .unwrap()
            .unwrap()
            .limits()
            .unwrap(),
        (90, 75)
    );
    assert!(custom.resolve(Some(0), 0).is_err());
}
#[test]
fn model_alias_revision_and_provider_cannot_rebind_frozen_policy() {
    for changed in [
        ModelIdentity {
            model_id: "alias".into(),
            ..model()
        },
        ModelIdentity {
            provider_id: "other".into(),
            ..model()
        },
        ModelIdentity {
            revision: Some("new".into()),
            ..model()
        },
    ] {
        assert!(
            execution_profile(&model(), AgentExecutionMode::DeepSeekNonThinking)
                .unwrap()
                .with_context_budget_policy(&changed, NativeContextBudgetPolicy::REVIEWED_DEFAULT)
                .is_err()
        );
    }
    for policy in [
        NativeContextBudgetPolicy {
            scope: research_agent_core::context::accounting::AutoCompactionScope::Total,
            effective_percent: 0,
            configured_trigger_tokens: None,
        },
        NativeContextBudgetPolicy {
            scope: research_agent_core::context::accounting::AutoCompactionScope::Total,
            effective_percent: 101,
            configured_trigger_tokens: None,
        },
        NativeContextBudgetPolicy {
            scope: research_agent_core::context::accounting::AutoCompactionScope::Total,
            effective_percent: 95,
            configured_trigger_tokens: Some(0),
        },
    ] {
        assert!(
            execution_profile(&model(), AgentExecutionMode::DeepSeekNonThinking)
                .unwrap()
                .with_context_budget_policy(&model(), policy)
                .is_err()
        );
        assert!(policy.resolve(None, 100).is_err());
    }
}
