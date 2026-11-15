//! Main-turn continuation follows Codex; explicit child budgets remain finite.
use super::*;
use research_agent_core::{model::*, protocol::EventPayload, runner::*};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Stream(VecDeque<ModelStreamEvent>);
impl ModelStream for Stream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move { Ok(self.0.pop_front()) })
    }
}
struct Reader {
    calls: AtomicUsize,
    pages: usize,
    model: ModelIdentity,
}
impl ModelTransport for Reader {
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: vec![],
        }
    }
    fn start(&self, _: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            let index = self.calls.fetch_add(1, Ordering::SeqCst);
            let mut events = VecDeque::from([ModelStreamEvent::Started {
                response_id: format!("response-{index}"),
                actual_model: self.model.clone(),
            }]);
            if index < self.pages {
                let call_id = format!("read-{index}");
                events.extend([
                    ModelStreamEvent::TextDelta("retrieving evidence ".repeat(100)),
                    ModelStreamEvent::ToolCallStarted {
                        call_id: call_id.clone(),
                        name: "read".into(),
                    },
                    ModelStreamEvent::ToolArgumentsDelta {
                        call_id: call_id.clone(),
                        delta: "{}".into(),
                    },
                    ModelStreamEvent::ToolCallCompleted { call_id },
                    ModelStreamEvent::Completed {
                        reason: FinishReason::ToolCalls,
                        end_turn: None,
                    },
                ]);
            } else {
                events.extend([
                    ModelStreamEvent::TextDelta("Algorithm explained from evidence".into()),
                    ModelStreamEvent::Completed {
                        reason: FinishReason::Stop,
                        end_turn: None,
                    },
                ]);
            }
            Ok(Box::new(Stream(events)) as Box<dyn ModelStream>)
        })
    }
}
struct Tools;
impl ToolExecutor for Tools {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        Ok(())
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async {
            Ok(ToolOutput {
                content: "structured evidence ".repeat(800),
                is_error: false,
            })
        })
    }
}
struct Sink;
impl TurnEventSink for Sink {
    fn emit(&self, _: EventPayload) -> RunFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}
struct Ids;
impl RunIdSource for Ids {
    fn next_id(&self) -> uuid::Uuid {
        uuid::Uuid::new_v4()
    }
}
#[tokio::test]
async fn main_turn_finishes_after_many_pages_and_explicit_call_budgets_still_apply() {
    let model = ModelIdentity {
        provider_id: "openai".into(),
        model_id: "gpt-5.6-terra".into(),
        revision: None,
    };
    let profile = execution_profile(&model, AgentExecutionMode::ChatGptDefault).unwrap();
    assert!(profile.limits.model_calls.is_none());
    assert!(profile.limits.tool_calls.is_none());
    assert!(profile.limits.output_bytes.is_none());
    // Exceed both former sampling caps, the old tool count and cumulative text
    // allowance. Single responses/results remain below their independent limits.
    for (pages, limits, expected_calls, success) in [
        (160, profile.limits, 161, true),
        (10, profile.limits.with_call_budget(3, 10), 3, false),
        (10, profile.limits.with_call_budget(10, 2), 3, false),
    ] {
        let transport = Reader {
            calls: AtomicUsize::new(0),
            pages,
            model: model.clone(),
        };
        let runner = TurnRunner {
            transport: &transport,
            tools: &Tools,
            sink: &Sink,
            ids: &Ids,
            limits,
        };
        let request = ModelRequest {
            captured_history: None,
            request_id: uuid::Uuid::new_v4(),
            model: model.clone(),
            messages: vec![ModelMessage::User {
                content: vec![ContentPart::Text("Explain the algorithm".into())],
                source_item_id: None,
            }],
            tools: vec![ToolDefinition {
                namespace: None,
                name: "read".into(),
                description: "read a bounded page".into(),
                parameters: serde_json::json!({"type":"object"}),
            }],
            max_output_tokens: 4096,
            parallel_tool_calls: false,
            reasoning_effort: None,
        };
        let result = runner.run(uuid::Uuid::new_v4(), request).await;
        if success {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(RunError::Budget)));
        }
        assert_eq!(transport.calls.load(Ordering::SeqCst), expected_calls);
    }
}
