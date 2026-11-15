use super::*;
use research_agent_core::{
    context::{accounting::ContextTokenEstimator, compaction_strategy::CompactionBudget},
    model::ToolDefinition,
    protocol::ModelIdentity,
};

struct Bytes;
impl ContextTokenEstimator for Bytes {
    fn estimate(
        &self,
        _: &[research_agent_core::model::ModelMessage],
        _: &[ToolDefinition],
    ) -> Option<u64> {
        Some(1)
    }
    fn estimate_compaction(&self, output: &CompactionOutput) -> Option<u64> {
        let CompactionOutput::RemoteWindow(items) = output else {
            return None;
        };
        Some(serde_json::to_vec(items).unwrap().len() as u64)
    }
}
fn fixture() -> (ModelRequest, ContextAccounting) {
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "producer".into(),
        revision: Some("exact".into()),
    };
    let request = ModelRequest {
        captured_history: None,
        request_id: Uuid::new_v4(),
        model: model.clone(),
        messages: vec![],
        tools: vec![],
        max_output_tokens: 1,
        parallel_tool_calls: false,
        reasoning_effort: None,
    };
    let accounting = ContextAccounting {
        model,
        budget: Some(CompactionBudget {
            context_window_tokens: 1200,
            effective_percent: 80,
            configured_trigger_tokens: None,
            output_reserve_tokens: 1,
        }),
        unknown_window_trigger_tokens: None,
        estimator: Arc::new(Bytes),
    };
    (request, accounting)
}
fn output(id: &str) -> Value {
    json!({"type":"function_call_output","id":id,"call_id":id,"output":"large".repeat(100),"success":false,"metadata":{"kept":true}})
}

#[test]
fn only_contiguous_tail_is_rewritten_until_the_producer_window_fits() {
    let (request, accounting) = fixture();
    let original = json!({"input":[{"role":"developer","content":"rules"},output("earlier"),output("last"),{"type":"compaction_trigger"}]});
    let mut body = original.clone();
    trim_body(&mut body, &request, Some(&accounting)).unwrap();
    assert_eq!(body["input"][1], original["input"][1]);
    assert_eq!(body["input"][2]["output"], TRUNCATED);
    for key in ["id", "call_id", "success", "metadata"] {
        assert_eq!(body["input"][2][key], original["input"][2][key]);
    }
    assert_eq!(body["input"][3], original["input"][3]);
    assert_eq!(original["input"][2], output("last"));
    let mut other = fixture().1;
    other.budget.as_mut().unwrap().context_window_tokens = 10000;
    let mut fresh = original.clone();
    trim_body(&mut fresh, &request, Some(&other)).unwrap();
    assert_eq!(
        fresh, original,
        "later attempt starts from the source, not the earlier rewrite"
    );
}

#[test]
fn unsupported_tail_item_is_a_boundary_and_search_status_remains_original() {
    let (request, mut accounting) = fixture();
    accounting.budget.as_mut().unwrap().context_window_tokens = 200;
    let original = json!({"input":[output("earlier"),{"role":"user","content":"boundary"},{"type":"tool_search_output","call_id":"search","tools":[{"schema":"large".repeat(100)}],"status":"completed","execution":"server","metadata":42}]});
    let mut body = original.clone();
    trim_body(&mut body, &request, Some(&accounting)).unwrap();
    assert_eq!(body["input"][0], original["input"][0]);
    assert_eq!(body["input"][1], original["input"][1]);
    assert_eq!(body["input"][2]["tools"], json!([]));
    for key in ["call_id", "status", "execution", "metadata"] {
        assert_eq!(body["input"][2][key], original["input"][2][key]);
    }
    assert!(
        serde_json::to_vec(&body).unwrap().len() > 160,
        "rewriting need not guarantee fit"
    );
}

#[test]
fn unknown_capacity_unknown_estimator_and_foreign_model_do_not_guess() {
    let (request, mut accounting) = fixture();
    let original = json!({"input":[output("one")]});
    let mut body = original.clone();
    accounting.budget = None;
    trim_body(&mut body, &request, Some(&accounting)).unwrap();
    assert_eq!(body, original);
    accounting.model.revision = None;
    assert!(matches!(
        trim_body(&mut body, &request, Some(&accounting)),
        Err(ModelError::InvalidRequest)
    ));
    struct Unknown;
    impl ContextTokenEstimator for Unknown {
        fn estimate(
            &self,
            _: &[research_agent_core::model::ModelMessage],
            _: &[ToolDefinition],
        ) -> Option<u64> {
            Some(1)
        }
    }
    let mut accounting = fixture().1;
    accounting.estimator = Arc::new(Unknown);
    trim_body(&mut body, &request, Some(&accounting)).unwrap();
    assert_eq!(body, original);
}

#[test]
fn rewriting_custom_output_preserves_the_local_error_envelope() {
    let (request, mut accounting) = fixture();
    accounting.budget.as_mut().unwrap().context_window_tokens = 200;
    let mut body = json!({"input":[{"type":"custom_tool_call_output","call_id":"custom","output":json!({"is_error":true,"content":"large".repeat(100)}).to_string(),"success":false}]});
    trim_body(&mut body, &request, Some(&accounting)).unwrap();
    let envelope: Value =
        serde_json::from_str(body["input"][0]["output"].as_str().unwrap()).unwrap();
    assert_eq!(envelope, json!({"is_error":true,"content":TRUNCATED}));
    assert_eq!(body["input"][0]["success"], false);
}
