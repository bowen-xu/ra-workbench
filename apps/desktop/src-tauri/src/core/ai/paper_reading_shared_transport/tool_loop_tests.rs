use super::*;
use research_agent_core::model::{FinishReason, ModelFuture, ModelStream, ModelStreamEvent};
use research_agent_core::runner::ToolOutput;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Stream(VecDeque<ModelStreamEvent>);
impl ModelStream for Stream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async { Ok(self.0.pop_front()) })
    }
}
struct Transport {
    requests: Mutex<Vec<ModelRequest>>,
    responses: Mutex<VecDeque<Vec<ModelStreamEvent>>>,
}
impl ModelTransport for Transport {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.requests.lock().unwrap().push(request);
        let events = self.responses.lock().unwrap().pop_front().unwrap();
        Box::pin(async move { Ok(Box::new(Stream(events.into())) as Box<dyn ModelStream>) })
    }
}
struct RepairTools(AtomicUsize);
impl ToolExecutor for RepairTools {
    fn validate(&self, _: &research_agent_core::model::ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn authorize_and_execute(
        &self,
        call: research_agent_core::model::ToolCall,
    ) -> RunFuture<'_, ToolOutput> {
        let index = self.0.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            call.name,
            if index == 0 {
                "research_read_file"
            } else {
                "research_apply_patch"
            }
        );
        Box::pin(async move {
            Ok(ToolOutput{content:match index{0=>"{\"text\":\"\",\"revision\":0}",1=>"{\"status\":\"invalid\",\"draftRevision\":1,\"errors\":[\"evidenceQuotes missing\"]}",_=>"{\"status\":\"applied\",\"revision\":1}"}.into(),is_error:index==1})
        })
    }
}
fn model() -> research_agent_core::protocol::ModelIdentity {
    research_agent_core::protocol::ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "tool-repair".into(),
        revision: None,
    }
}
fn response(index: usize, name: Option<&str>) -> Vec<ModelStreamEvent> {
    let mut events = vec![ModelStreamEvent::Started {
        response_id: format!("response-{index}"),
        actual_model: model(),
    }];
    if let Some(name) = name {
        let call_id = format!("call-{index}");
        events.extend([
            ModelStreamEvent::ToolCallStarted {
                call_id: call_id.clone(),
                name: name.into(),
            },
            ModelStreamEvent::ToolArgumentsDelta {
                call_id: call_id.clone(),
                delta: json!({"patch":"a valid exact patch"}).to_string(),
            },
            ModelStreamEvent::ToolCallCompleted { call_id },
            ModelStreamEvent::Completed {
                reason: FinishReason::ToolCalls,
                end_turn: None,
            },
        ]);
    } else {
        events.extend([
            ModelStreamEvent::TextDelta("Saved successfully".into()),
            ModelStreamEvent::Completed {
                reason: FinishReason::Stop,
                end_turn: None,
            },
        ]);
    }
    events
}

#[tokio::test]
async fn structured_transport_allows_read_invalid_write_and_same_turn_repair() {
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        responses: Mutex::new(
            vec![
                response(0, Some("research_read_file")),
                response(1, Some("research_apply_patch")),
                response(2, Some("research_apply_patch")),
                response(3, None),
            ]
            .into(),
        ),
    });
    let tools = Arc::new(RepairTools(AtomicUsize::new(0)));
    let request = ModelRequest {
        captured_history: None,
        request_id: Uuid::new_v4(),
        model: model(),
        messages: vec![ModelMessage::User {
            content: vec![ContentPart::Text("Edit this managed object".into())],
            source_item_id: None,
        }],
        tools: ["research_read_file", "research_apply_patch"]
            .map(|name| ToolDefinition {
                namespace: None,
                name: name.into(),
                description: name.into(),
                parameters: json!({"type":"object"}),
            })
            .to_vec(),
        max_output_tokens: 8000,
        parallel_tool_calls: false,
        reasoning_effort: None,
    };
    let result = consume_vision_stream(
        transport.clone(),
        request,
        None,
        None,
        "fixture",
        None,
        None,
        tools.clone(),
    )
    .await
    .unwrap();
    assert_eq!(result.content, "Saved successfully");
    assert_eq!(result.input_tokens, None);
    assert_eq!(result.output_tokens, None);
    assert_eq!(tools.0.load(Ordering::SeqCst), 3);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[2].messages.iter().any(|message| matches!(message,ModelMessage::ToolResult{content,..} if content.contains("evidenceQuotes missing"))));
}
