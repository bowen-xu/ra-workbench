use super::*;
use crate::agent::{
    cancellation::{AgentCancellation, run_cancellable_turn},
    composed_tools::RejectingToolExecutor,
};
use research_agent_core::{
    model::*,
    protocol::EventPayload,
    runner::{RunFuture, RunIdSource, RunLimits, TurnEventSink, TurnRunner},
};
use std::{collections::VecDeque, time::Duration};

struct Transport;
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
        Box::pin(async move {
            Ok(Box::new(Stream(VecDeque::from([
                ModelStreamEvent::Started {
                    response_id: "fixture".into(),
                    actual_model: request.model,
                },
                ModelStreamEvent::ToolCallStarted {
                    call_id: "parse".into(),
                    name: "research_read_source".into(),
                },
                ModelStreamEvent::ToolArgumentsDelta {
                    call_id: "parse".into(),
                    delta: json!({"fileId":"file"}).to_string(),
                },
                ModelStreamEvent::ToolCallCompleted {
                    call_id: "parse".into(),
                },
                ModelStreamEvent::Completed {
                    reason: FinishReason::ToolCalls,
                    end_turn: None,
                },
            ]))) as Box<dyn ModelStream>)
        })
    }
}
struct Stream(VecDeque<ModelStreamEvent>);
impl ModelStream for Stream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async { Ok(self.0.pop_front()) })
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
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}

#[tokio::test]
async fn canceled_pdf_tool_detaches_its_waiter_without_restarting_a_shared_conversion() {
    for other_waiter in [false, true] {
        let (_root, tools, parser) = pdf_tools();
        let preparation = tools.store.docling.get().unwrap().clone();
        let registered = registration(
            tools.store.clone(),
            tools.record.clone(),
            OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
        );
        let (definitions, executor) = registered.into_parts();
        let read = call("research_read_source", json!({"fileId":"file"}));
        assert!(
            executor.cancellation_safe_to_drop(&read),
            "real TextTools/composed chain forwards the proven policy"
        );
        assert!(!executor.supports_parallel(&read));
        let request = ModelRequest {
            captured_history: None,
            request_id: Uuid::new_v4(),
            model: tools.record.input.model.clone(),
            messages: vec![ModelMessage::User {
                content: vec![ContentPart::Text("read PDF".into())],
                source_item_id: None,
            }],
            tools: definitions,
            max_output_tokens: 128,
            parallel_tool_calls: false,
            reasoning_effort: None,
        };
        let cancellation = AgentCancellation::default();
        let mut run = Box::pin(run_cancellable_turn(
            TurnRunner {
                transport: &Transport,
                tools: executor.as_ref(),
                sink: &Sink,
                ids: &Ids,
                limits: RunLimits {
                    model_calls: Some(2),
                    tool_calls: Some(1),
                    output_bytes: Some(1000),
                    tool_output_bytes: 4096,
                    response: StreamLimits {
                        text_bytes: 1000,
                        argument_bytes: 1000,
                        tool_calls: 1,
                    },
                },
            },
            tools.record.turn_id,
            request,
            cancellation.clone(),
        ));
        tokio::select! {
            _ = parser.wait_started() => {},
            _ = &mut run => panic!("the fixture parser must stay pending"),
        }
        let mut shared = Box::pin(preparation.ensure(&tools.store, "paper", "file"));
        if other_waiter {
            tokio::select! { biased;
                _ = &mut shared => panic!("the shared conversion must stay pending"),
                _ = tokio::task::yield_now() => {},
            }
        }
        cancellation.cancel();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), run)
                .await
                .unwrap(),
            Err(RunError::Model(ModelError::Canceled))
        ));
        assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
        if other_waiter {
            parser.release.add_permits(1);
            assert!(shared.await.is_ok());
            assert_eq!(
                parser.calls.load(Ordering::SeqCst),
                1,
                "another owner keeps the same conversion"
            );
        } else {
            let count: u64 = tools
                .store
                .connection()
                .unwrap()
                .query_row("select count(*) from docling_parse_runs", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(
                count, 0,
                "the dropped parser did not write a completed cache result"
            );
        }
    }
}

#[test]
fn only_raw_pdf_tools_opt_in_without_parallel_or_other_resource_permissions() {
    let (_root, mut tools, _parser) = pdf_tools();
    for name in ["research_read_source", "research_search_sources"] {
        let call = call(name, json!({"fileId":"file"}));
        assert!(tools.cancellation_safe_to_drop(&call));
        assert!(!tools.supports_parallel(&call));
    }
    assert!(!tools.cancellation_safe_to_drop(&call(
        "research_read_source",
        json!({"fileId":"file","view":"structured"})
    )));
    assert!(!tools.cancellation_safe_to_drop(&call("research_list_sources", json!({}))));
    // A frozen paper-metadata grant is not an authorization to detach any
    // broader live resource tool, even if it belongs to the same publication.
    let metadata = resource_grants::grants(
        &tools.store.connection().unwrap(),
        &[Resource::PaperMetadata {
            resource_id: "paper".into(),
        }],
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    tools.record.input.context.extend(metadata);
    assert!(
        !tools.cancellation_safe_to_drop(&call("research_read_source", json!({"fileId":"file"})))
    );
}
