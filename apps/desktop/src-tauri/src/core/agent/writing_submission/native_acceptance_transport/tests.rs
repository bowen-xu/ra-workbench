use super::*;
use research_agent_core::{model::ToolDefinition, protocol::ModelIdentity};
use serde_json::{Value, json};

fn fixture() -> (tempfile::TempDir, Uuid, Value) {
    let root = tempfile::Builder::new()
        .prefix("research-assistant-writing-diff-agent-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let id = Uuid::new_v4();
    std::fs::write(
        root.path().join("native-acceptance.json"),
        json!({
            "dataDirectory":root.path(), "manuscriptId":id,
        })
        .to_string(),
    )
    .unwrap();
    let script = json!({"schemaVersion":1,"manuscriptId":id,"requiredInput":"latest unsaved manuscript",
        "steps":[{"kind":"tool","name":"research_apply_patch","arguments":{"operations":[]}},
            {"kind":"tool","name":"research_review_workspace_changes","arguments":{}},
            {"kind":"text","text":"Prepared changes."}]});
    write_script(root.path(), &script);
    (root, id, script)
}

fn write_script(root: &Path, script: &Value) {
    std::fs::write(
        root.join("native-acceptance-model.json"),
        script.to_string(),
    )
    .unwrap();
}

fn request(messages: Vec<ModelMessage>) -> ModelRequest {
    ModelRequest {
        captured_history: None,
        request_id: Uuid::new_v4(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
        messages,
        tools: ["research_apply_patch", "research_review_workspace_changes"]
            .into_iter()
            .map(|name| ToolDefinition {
                namespace: None,
                name: name.into(),
                description: "Synthetic only".into(),
                parameters: json!({}),
            })
            .collect(),
        max_output_tokens: 100,
        parallel_tool_calls: false,
        reasoning_effort: None,
    }
}

fn transport(root: &Path, id: Uuid) -> FixtureTransport {
    FixtureTransport {
        script: read_script(root, id).unwrap(),
        calls: AtomicUsize::new(0),
        capabilities: ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        },
        audit: root.join("native-acceptance-model-observations.jsonl"),
    }
}

#[test]
fn fixture_requires_explicit_flag_and_exact_synthetic_scope() {
    assert_eq!(enabled(None), Ok(false));
    assert_eq!(enabled(Some(std::ffi::OsStr::new("1"))), Ok(true));
    for value in ["", "0", "true"] {
        assert_eq!(
            enabled(Some(std::ffi::OsStr::new(value))),
            Err(ModelError::InvalidRequest)
        );
    }
    let (root, id, _) = fixture();
    assert!(read_script(root.path(), id).is_ok());
    assert!(matches!(
        read_script(root.path(), Uuid::new_v4()),
        Err(ModelError::InvalidRequest)
    ));
    let foreign = tempfile::tempdir().unwrap();
    assert!(matches!(
        read_script(foreign.path(), id),
        Err(ModelError::InvalidRequest)
    ));
    std::fs::write(
        root.path().join("native-acceptance.json"),
        json!({"dataDirectory":root.path(),"manuscriptId":Uuid::new_v4()}).to_string(),
    )
    .unwrap();
    assert!(matches!(
        read_script(root.path(), id),
        Err(ModelError::InvalidRequest)
    ));
}

#[test]
fn script_rejects_unknown_fields_tools_invalid_scope_and_unbounded_steps() {
    let (root, id, script) = fixture();
    let mut variants = vec![];
    for (key, value) in [
        ("schemaVersion", json!(2)),
        ("manuscriptId", json!(Uuid::new_v4())),
        ("requiredInput", json!("")),
        ("extra", json!(true)),
        ("pauseBeforeCall", json!(3)),
        ("pauseBeforeCall", json!(-1)),
        ("steps", json!([])),
        (
            "steps",
            json!([{"kind":"tool","name":"research_apply_patch","arguments":{}}]),
        ),
        ("steps", json!([{"kind":"text","text":""}])),
    ] {
        let mut changed = script.clone();
        changed[key] = value;
        variants.push(changed);
    }
    let mut tool = script.clone();
    tool["steps"][0]["name"] = json!("research_shell");
    variants.push(tool);
    let mut arguments = script.clone();
    arguments["steps"][0]["arguments"] = json!([]);
    variants.push(arguments);
    let mut extra = script.clone();
    extra["steps"][0]["extra"] = json!(true);
    variants.push(extra);
    let mut many = script.clone();
    many["steps"] = json!(vec![json!({"kind":"text","text":"answer"}); 9]);
    variants.push(many);
    for changed in variants {
        write_script(root.path(), &changed);
        assert!(matches!(
            read_script(root.path(), id),
            Err(ModelError::InvalidRequest)
        ));
    }
}

#[test]
fn reads_are_bounded_and_never_follow_symlink_files() {
    let (root, id, _) = fixture();
    let script_path = root.path().join("native-acceptance-model.json");
    std::fs::write(&script_path, vec![b' '; MAX_SCRIPT_BYTES as usize + 1]).unwrap();
    assert!(matches!(
        read_script(root.path(), id),
        Err(ModelError::InvalidRequest)
    ));
    std::fs::remove_file(&script_path).unwrap();
    std::os::unix::fs::symlink(root.path().join("native-acceptance.json"), &script_path).unwrap();
    assert!(matches!(
        read_script(root.path(), id),
        Err(ModelError::InvalidRequest)
    ));
    let audit = root
        .path()
        .join("native-acceptance-model-observations.jsonl");
    std::os::unix::fs::symlink(root.path().join("native-acceptance.json"), &audit).unwrap();
    assert!(open_file(&audit, FileAccess::Append).is_err());
}

#[tokio::test]
async fn native_requests_must_include_latest_input_and_successful_tool_results() {
    let (root, id, _) = fixture();
    let expected = "latest unsaved manuscript";
    for messages in [
        vec![ModelMessage::User {
            content: vec![ContentPart::Text("old manuscript".into())],
            source_item_id: None,
        }],
        vec![ModelMessage::Assistant {
            phase: None,
            content: vec![ContentPart::Text(expected.into())],
            tool_calls: vec![],
        }],
        vec![ModelMessage::ToolResult {
            call_id: "old".into(),
            content: expected.into(),
            is_error: false,
        }],
        vec![
            ModelMessage::System(expected.into()),
            ModelMessage::ToolResult {
                call_id: "bad".into(),
                content: "failed".into(),
                is_error: true,
            },
        ],
    ] {
        assert!(matches!(
            transport(root.path(), id).start(request(messages)).await,
            Err(ModelError::InvalidRequest)
        ));
    }
    let mut unapproved = request(vec![ModelMessage::System(expected.into())]);
    unapproved.tools.clear();
    assert!(matches!(
        transport(root.path(), id).start(unapproved).await,
        Err(ModelError::UnsupportedCapability)
    ));
    assert!(
        !root
            .path()
            .join("native-acceptance-model-observations.jsonl")
            .exists()
    );
}

#[tokio::test]
async fn frozen_script_emits_runner_events_and_records_only_scope_and_hashes() {
    let (root, id, script) = fixture();
    let fixture = transport(root.path(), id);
    // Changing the response fixture cannot change an already prepared native turn.
    write_script(root.path(), &json!({"invalid":true}));
    let input = script["requiredInput"].as_str().unwrap();
    let request = request(vec![ModelMessage::User {
        content: vec![ContentPart::Text(input.into())],
        source_item_id: None,
    }]);
    for expected in [
        FinishReason::ToolCalls,
        FinishReason::ToolCalls,
        FinishReason::Stop,
    ] {
        let mut stream = fixture.start(request.clone()).await.unwrap();
        assert!(matches!(
            stream.next().await.unwrap(),
            Some(ModelStreamEvent::Started { .. })
        ));
        let mut completion = None;
        while let Some(event) = stream.next().await.unwrap() {
            if let ModelStreamEvent::Completed { reason, .. } = event {
                completion = Some(reason);
            }
        }
        assert_eq!(completion, Some(expected));
    }
    assert!(matches!(
        fixture.start(request.clone()).await,
        Err(ModelError::InvalidRequest)
    ));
    let audit = std::fs::read_to_string(&fixture.audit).unwrap();
    assert!(!audit.contains(input));
    assert!(!audit.contains("Prepared changes."));
    let rows = audit
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 3);
    for (call, row) in rows.iter().enumerate() {
        assert_eq!(row.as_object().unwrap().len(), 4);
        assert_eq!(row["manuscriptId"], json!(id));
        assert_eq!(row["requestId"], json!(request.request_id));
        assert_eq!(row["call"], json!(call));
        assert_eq!(
            row["requiredInputSha256"],
            json!(format!("{:x}", Sha256::digest(input.as_bytes())))
        );
    }
}

#[test]
fn latest_input_matches_production_json_sources_without_normalizing_text() {
    let expected = "\\section{Method}\r\nLatest unsaved input.\n";
    let message = |text: &str| ModelMessage::User {
        content: vec![ContentPart::Text(format!(
            "Frozen research sources for this turn.\n{}",
            json!([{ "text":text }])
        ))],
        source_item_id: None,
    };
    assert!(has_input(&request(vec![message(expected)]), expected));
    assert!(!has_input(
        &request(vec![message("\\section{Method}\nLatest unsaved input.\n")]),
        expected
    ));
    assert!(!has_input(
        &request(vec![message("\\section{Method}\r\nOld input.\n")]),
        expected
    ));
}
