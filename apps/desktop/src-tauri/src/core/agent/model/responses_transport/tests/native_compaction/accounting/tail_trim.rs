use super::*;
struct LargeTool;
impl ToolExecutor for LargeTool {
    fn validate(&self, _: &research_agent_core::model::ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn authorize_and_execute(
        &self,
        _: research_agent_core::model::ToolCall,
    ) -> RunFuture<'_, ToolOutput> {
        Box::pin(async {
            Ok(ToolOutput {
                content: "large evidence".repeat(2000),
                is_error: false,
            })
        })
    }
}
#[tokio::test]
async fn remote_midturn_trims_only_the_request_copy_of_large_tool_output() {
    for legacy in [false, true] {
        exercise(legacy).await;
    }
}

async fn exercise(legacy: bool) {
    let fixture = Fixture::new().await;
    let call = json!({"type":"function_call","id":"read-item","call_id":"read1","name":"read","arguments":"{}"});
    let frames = [
        json!({"type":"response.created","response":{"id":"r1","model":"fixture-model","headers":{"OpenAI-Model":"fixture-model"}}}),
        json!({"type":"response.output_item.done","item":call}),
        json!({"type":"response.completed","response":{"id":"r1","model":"fixture-model","status":"completed",
            "output":[call],"usage":{"input_tokens":1500,"output_tokens":400}}}),
    ].into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
    let (url, peer) = sequential_server(vec![
        ok(&frames),
        if legacy {
            super::super::legacy::json_response(&super::super::legacy::full_window("small state"))
        } else {
            ok(&events(&[opaque()], true))
        },
        ok(&body()),
    ])
    .await;
    let record = fixture.record("small submission").await;
    let inner = fixture.transport(&record, &url);
    let mut execution = if legacy {
        super::super::legacy::legacy_execution(&fixture, &record, &url)
    } else {
        fixture.execution(&record, inner.clone())
    };
    execution.limits.tool_output_bytes = 40_000;
    account(&mut execution, inner, 2000);
    execution.host_tools = OwnedToolRegistration::new(
        vec![research_agent_core::model::ToolDefinition {
            namespace: None,
            name: "read".into(),
            description: "Read authorized evidence".into(),
            parameters: json!({"type":"object"}),
        }],
        Arc::new(LargeTool),
    );
    assert_eq!(
        fixture
            .start(&record, execution, false)
            .await
            .task
            .wait()
            .await,
        TaskCompletion::Completed
    );
    let window = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(window.phase, CompactionPhase::MidTurn);
    let projection = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    let turn = projection.turn(record.turn_id).unwrap();
    assert!(
        turn.items
            .values()
            .any(|item| item.text == "large evidence".repeat(2000))
    );
    let requests = peer.await.unwrap();
    let result = requests[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap();
    assert_eq!(result["call_id"], "read1");
    assert_eq!(
        result["output"],
        "Output exceeded the available model context and was truncated"
    );

    if !legacy {
        assert!(
            requests[1]["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "compaction_trigger")
        );
        assert!(requests[2]["input"].as_array().unwrap().contains(&opaque()));
    }
}
