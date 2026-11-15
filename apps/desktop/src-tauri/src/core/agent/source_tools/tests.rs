use super::*;
use research_agent_core::{
    protocol::ModelIdentity,
    submission::{ContextSourceKind, ContextText, SubmissionInput},
};
use uuid::Uuid;

#[test]
fn only_frozen_submission_reads_opt_into_parallel_and_drop_safe_dispatch() {
    let record = record();
    let borrowed = SubmissionTools { record: &record };
    let owned = OwnedSubmissionTools::new(record.clone());
    for tools in [&borrowed as &dyn ToolExecutor, &owned as &dyn ToolExecutor] {
        for name in [LIST, READ, SEARCH] {
            let call = call(name, json!({}));
            assert!(tools.supports_parallel(&call));
            assert!(tools.cancellation_safe_to_drop(&call));
        }
        let call = call("research_exec_command", json!({}));
        assert!(!tools.supports_parallel(&call));
        assert!(!tools.cancellation_safe_to_drop(&call));
    }
}

#[tokio::test]
async fn source_executor_accepts_receipts_only_for_its_registered_names() {
    let record = record();
    let tools = SubmissionTools { record: &record };
    for name in [LIST, READ, SEARCH] {
        tools.result_committed(call(name, json!({}))).await.unwrap();
    }
    assert!(matches!(
        tools.result_committed(call("exec", json!({}))).await,
        Err(RunError::ToolRejected)
    ));
}

#[tokio::test]
async fn owned_source_executor_matches_borrowed_snapshot_behavior() {
    let record = record();
    let tools = OwnedSubmissionTools::new(record.clone());
    let owned_names = tools
        .definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect::<Vec<_>>();
    let borrowed_names = definitions(&record)
        .into_iter()
        .map(|definition| definition.name)
        .collect::<Vec<_>>();
    assert_eq!(owned_names, borrowed_names);
    let result = tools
        .authorize_and_execute(call(READ, json!({"sourceId":"S1","maxChars":2})))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(
        value["contextSnapshotId"],
        record.context_snapshot_id.to_string()
    );
    assert_eq!(value["text"], "甲🧬");
}

fn record() -> SubmissionRecord {
    SubmissionRecord {
        conversation_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
        input: SubmissionInput {
            remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
            text: "Read".into(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![ContextText {
                source_id: "S1".into(),
                kind: ContextSourceKind::Manuscript,
                resource_id: "draft".into(),
                revision: "v1".into(),
                anchor: Some("line:4".into()),
                text: "甲🧬e\u{301}乙".into(),
            }],
        },
    }
}
fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "call".into(),
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
async fn search_positions_can_be_read_without_losing_unicode_or_snapshot_identity() {
    let mut record = record();
    record.input.context[0].text = "甲🧬e\u{301}乙🧬e\u{301}末".into();
    let tools = SubmissionTools { record: &record };
    let result = tools
        .authorize_and_execute(call(SEARCH, json!({"query":"🧬e\u{301}"})))
        .await
        .unwrap();
    let found: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(
        found["contextSnapshotId"],
        record.context_snapshot_id.to_string()
    );
    assert_eq!(found["total"], 2);
    assert_eq!(found["matches"][0]["startChar"], 1);
    assert_eq!(found["matches"][1]["startChar"], 5);
    for item in found["matches"].as_array().unwrap() {
        let read = tools
            .evaluate(&call(
                READ,
                json!({"sourceId":item["sourceId"],
            "startChar":item["startChar"],"maxChars":3}),
            ))
            .unwrap();
        assert_eq!(read["text"], "🧬e\u{301}");
        assert_eq!(item["revision"], read["revision"]);
        assert_eq!(item["anchor"], read["anchor"]);
    }
}

#[test]
fn search_pages_literal_matches_and_rejects_scope_expansion() {
    let mut record = record();
    record.input.context[0].text = "a.* ".repeat(21);
    let mut second = record.input.context[0].clone();
    second.source_id = "S2".into();
    second.text = "a.*".into();
    record.input.context.push(second);
    let tools = SubmissionTools { record: &record };
    let first = tools
        .evaluate(&call(SEARCH, json!({"query":".*"})))
        .unwrap();
    assert_eq!(first["total"], 22);
    assert_eq!(first["nextOffset"], 20);
    assert_eq!(first["matches"].as_array().unwrap().len(), 20);
    let last = tools
        .evaluate(&call(SEARCH, json!({"query":".*","offset":20})))
        .unwrap();
    assert_eq!(last["matches"].as_array().unwrap().len(), 2);
    assert_eq!(last["matches"][1]["sourceId"], "S2");
    assert!(last["nextOffset"].is_null());
    assert_eq!(
        tools
            .evaluate(&call(SEARCH, json!({"query":".*","sourceId":"S2"})))
            .unwrap()["total"],
        1
    );
    assert_eq!(
        tools.evaluate(&call(SEARCH, json!({"query":"A"}))).unwrap()["total"],
        0
    );
    for args in [
        json!({"query":""}),
        json!({"query":"a","sourceId":"outside"}),
        json!({"query":"a","path":"/tmp"}),
        json!({"query":"a","offset":23}),
        json!({"query":"a","offset":-1}),
        json!({"query":"🧬".repeat(129)}),
    ] {
        assert!(tools.validate(&call(SEARCH, args)).is_err());
    }
}
#[tokio::test]
async fn unicode_pages_preserve_snapshot_identity_and_reconstruct_exact_text() {
    let record = record();
    let tools = SubmissionTools { record: &record };
    let mut text = String::new();
    let mut offset = 0;
    loop {
        let result = tools
            .authorize_and_execute(call(
                READ,
                json!({"sourceId":"S1","startChar":offset,"maxChars":2}),
            ))
            .await
            .unwrap();
        assert!(!result.is_error);
        let value: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(
            value["contextSnapshotId"],
            record.context_snapshot_id.to_string()
        );
        assert_eq!(value["revision"], "v1");
        assert_eq!(value["anchor"], "line:4");
        text.push_str(value["text"].as_str().unwrap());
        let Some(next) = value["nextChar"].as_u64() else {
            break;
        };
        offset = next;
    }
    assert_eq!(text, record.input.context[0].text);
}
#[test]
fn foreign_sources_invalid_ranges_and_extra_authority_are_rejected() {
    let record = record();
    let tools = SubmissionTools { record: &record };
    for args in [
        json!({"sourceId":"foreign"}),
        json!({"sourceId":"S1","path":"/tmp/anything"}),
        json!({"sourceId":"S1","startChar":6}),
        json!({"sourceId":"S1","startChar":-1}),
        json!({"sourceId":"S1","maxChars":0}),
        json!({"sourceId":"S1","maxChars":8001}),
    ] {
        assert!(tools.validate(&call(READ, args)).is_err());
    }
    assert!(tools.validate(&call("shell", json!({}))).is_err());
}
#[test]
fn source_listing_pages_do_not_include_source_text_and_empty_context_has_no_tools() {
    let mut record = record();
    for index in 1..25 {
        let mut source = record.input.context[0].clone();
        source.source_id = format!("S{}", index + 1);
        record.input.context.push(source);
    }
    let tools = SubmissionTools { record: &record };
    let first = tools.evaluate(&call(LIST, json!({}))).unwrap();
    assert_eq!(first["sources"].as_array().unwrap().len(), 20);
    assert_eq!(first["nextOffset"], 20);
    assert!(first["sources"][0].get("text").is_none());
    let second = tools.evaluate(&call(LIST, json!({"offset":20}))).unwrap();
    assert_eq!(second["sources"].as_array().unwrap().len(), 5);
    assert!(second["nextOffset"].is_null());
    record.input.context.clear();
    assert!(definitions(&record).is_empty());
    assert!(
        SubmissionTools { record: &record }
            .validate(&call(READ, json!({"sourceId":"S1"})))
            .is_err()
    );
}

#[test]
fn read_receipts_keep_names_and_exact_unicode_line_ranges_for_the_read_view() {
    let mut record = record();
    record.input.context[0].text = "一🧪\nsecond\nlast".into();
    record.input.context[0].anchor = Some(
        json!({"label":"original.md","sourceUrl":"source://writings/draft/original.md"})
            .to_string(),
    );
    let partial = evaluate(
        &record,
        &call(READ, json!({"sourceId":"S1","startChar":3,"maxChars":7})),
    )
    .unwrap();
    assert_eq!(partial["name"], "original.md");
    assert_eq!(partial["sourceUrl"], "source://writings/draft/original.md");
    assert_eq!(partial["startLine"], 2);
    assert_eq!(partial["endLine"], 2);
    let full = evaluate(
        &record,
        &call(READ, json!({"sourceId":"S1","maxChars":8000})),
    )
    .unwrap();
    assert_eq!(full["startLine"], 1);
    assert_eq!(full["endLine"], 3);
    assert!(full["nextChar"].is_null());
    assert_eq!(full["readViewId"], partial["readViewId"]);
    record.input.context[0].text.push('!');
    let changed = evaluate(
        &record,
        &call(READ, json!({"sourceId":"S1","maxChars":8000})),
    )
    .unwrap();
    assert_ne!(changed["readViewId"], full["readViewId"]);
}
