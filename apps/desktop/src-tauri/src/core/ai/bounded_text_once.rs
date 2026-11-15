//! One model stream, no Agent runner, tools, compaction or automatic resubmission.
use super::{
    error::AiError,
    paper_reading::ProviderInput,
    paper_reading_shared_transport::{map_model_error, provider_messages},
    provider::CancellationSignal,
};
use crate::agent::model::{
    ChatRequestPolicy, OutputTokenParameter, PreparedAgentModel,
    responses_continuation::ResponsesContinuation,
};
use research_agent_core::model::{
    ModelCapabilities, ModelError, ModelOutputItem, ModelRequest, ModelResponse, ModelStreamEvent,
    ModelTransport, ResponseAssembler, StreamLimits,
};
use std::sync::Arc;
use uuid::Uuid;

pub(super) async fn call_once(
    prepared: PreparedAgentModel,
    input: ProviderInput,
    signal: &CancellationSignal,
) -> Result<ModelResponse, AiError> {
    let model = prepared.identity().clone();
    let effort = prepared.reasoning_effort().map(str::to_owned);
    let capabilities = ModelCapabilities {
        vision: false,
        tools: false,
        parallel_tools: false,
        reasoning_efforts: effort.clone().into_iter().collect(),
    };
    let request_id = Uuid::new_v4();
    let request = ModelRequest {
        captured_history: None,
        request_id,
        model: model.clone(),
        messages: provider_messages(input, true)?,
        tools: vec![],
        max_output_tokens: 700,
        parallel_tool_calls: false,
        reasoning_effort: effort,
    };
    request
        .validate(capabilities.clone())
        .map_err(map_model_error)?;
    let transport: Arc<dyn ModelTransport> = if prepared.is_chatgpt() {
        let receipts = Arc::new(
            ResponsesContinuation::new(request_id, model, 32 * 1024).map_err(map_model_error)?,
        );
        Arc::new(
            prepared
                .into_chatgpt_transport(capabilities, receipts)
                .map_err(|_| AiError::Provider("invalid_request"))?,
        )
    } else {
        Arc::new(
            prepared
                .into_chat_transport_with_policy(
                    capabilities,
                    OutputTokenParameter::MaxTokens,
                    ChatRequestPolicy::JsonObject,
                )
                .map_err(|_| AiError::Provider("invalid_request"))?,
        )
    };
    signal
        .race(consume(transport.as_ref(), request))
        .await
        .map_err(|_| AiError::Canceled)?
}

async fn consume(
    transport: &dyn ModelTransport,
    request: ModelRequest,
) -> Result<ModelResponse, AiError> {
    transport
        .authorize_request(&request)
        .await
        .map_err(map_model_error)?;
    let requested_model = request.model.clone();
    let mut stream = transport.start(request).await.map_err(map_model_error)?;
    // This use case accepts text only; no executor or runner is constructed.
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 16 * 1024,
        argument_bytes: 1,
        tool_calls: 1,
    })
    .map_err(map_model_error)?
    .with_requested_model(requested_model);
    while let Some(event) = stream.next().await.map_err(map_model_error)? {
        if matches!(event, ModelStreamEvent::ToolCallStarted { .. })
            || matches!(&event,ModelStreamEvent::OutputItemDone(item)
                if matches!(item.normalized,Some(ModelOutputItem::ToolCall(_))))
        {
            return Err(map_model_error(ModelError::InvalidStream));
        }
        assembler.push(event).map_err(map_model_error)?;
    }
    assembler.finish().map_err(map_model_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::{
        model::{FinishReason, ModelFuture, ModelStream, ModelStreamEvent},
        protocol::ModelIdentity,
    };
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
        calls: AtomicUsize,
        tool: Option<ModelStreamEvent>,
    }
    impl ModelTransport for Transport {
        fn capabilities(&self) -> ModelCapabilities {
            ModelCapabilities {
                vision: false,
                tools: false,
                parallel_tools: false,
                reasoning_efforts: vec![],
            }
        }
        fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert!(request.tools.is_empty());
            let event = self
                .tool
                .clone()
                .unwrap_or_else(|| ModelStreamEvent::TextDelta("Answer".into()));
            let reason = if self.tool.is_some() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            };
            Box::pin(async move {
                Ok(Box::new(Stream(VecDeque::from([
                    ModelStreamEvent::Started {
                        response_id: "r".into(),
                        actual_model: request.model,
                    },
                    event,
                    ModelStreamEvent::Completed {
                        reason,
                        end_turn: None,
                    },
                ]))) as Box<dyn ModelStream>)
            })
        }
    }
    #[tokio::test]
    async fn single_response_never_enters_a_tool_loop_or_retries() {
        use research_agent_core::model::{CompletedModelItem, ToolCallIntent};
        for tool in [
            None,
            Some(ModelStreamEvent::ToolCallStarted {
                call_id: "bad".into(),
                name: "read_file".into(),
            }),
            Some(ModelStreamEvent::OutputItemDone(Box::new(
                CompletedModelItem {
                    canonical_item_id: None,
                    native: None,
                    normalized: Some(ModelOutputItem::ToolCall(ToolCallIntent {
                        call_id: "bad".into(),
                        name: "read_file".into(),
                        raw_arguments_json: String::new(),
                        namespace: None,
                    })),
                },
            ))),
        ] {
            let transport = Transport {
                calls: AtomicUsize::new(0),
                tool: tool.clone(),
            };
            let request = ModelRequest {
                captured_history: None,
                request_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "test".into(),
                    revision: None,
                },
                messages: vec![],
                tools: vec![],
                max_output_tokens: 700,
                parallel_tool_calls: false,
                reasoning_effort: None,
            };
            let result = consume(&transport, request).await;
            assert_eq!(result.is_err(), tool.is_some());
            assert_eq!(transport.calls.load(Ordering::Relaxed), 1);
        }
    }
}
