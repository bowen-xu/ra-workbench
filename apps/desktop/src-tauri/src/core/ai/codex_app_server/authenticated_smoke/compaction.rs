//! Opt-in provider acceptance of a real opaque candidate and its next request.
//! Synthetic content only; this does not prove SQLite installation or fidelity.
use super::*;
use crate::agent::execution_profile::NativeCompactionPolicy;
use crate::agent::model::{
    responses_continuation::ResponsesContinuation,
    responses_transport::ResponsesV2CompactionStrategy,
};
use research_agent_core::context::compaction_strategy::{
    CompactionFailure, CompactionOutput, CompactionPhase, CompactionRequest, prepare_compaction,
};

const MAX_BYTES: usize = 1024 * 1024;

#[tokio::test]
#[ignore = "real compaction and model calls; requires explicit opt-in, app-owned login, and closed development application"]
async fn authenticated_chatgpt_v2_compaction_continues_with_returned_opaque_item() {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, _work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        compact_and_continue(bridge.clone()),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("synthetic compaction acceptance timed out")
        .expect("synthetic compaction continuation failed");
}

async fn compact_and_continue(bridge: CodexAppServerManager) -> Result<(), CompactionFailure> {
    let model = ModelIdentity {
        provider_id: "openai".into(),
        model_id: "gpt-5.6-terra".into(),
        revision: None,
    };
    let capabilities = ModelCapabilities {
        vision: false,
        tools: false,
        parallel_tools: false,
        reasoning_efforts: vec![],
    };
    let binding = NativeCompactionPolicy::ChatGptResponsesV2.binding(model.clone());
    let transport = Arc::new(chatgpt_transport(
        bridge.clone(),
        Arc::new(ReceiptCounter::default()),
        capabilities.clone(),
    )?);
    let strategy = ResponsesV2CompactionStrategy::new(transport, binding.clone())?;
    let operation_id = Uuid::new_v4();
    let candidate = prepare_compaction(
        &strategy,
        CompactionRequest {
            operation_id,
            conversation_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            history_revision: 1,
            phase: CompactionPhase::Manual,
            binding,
            source: ModelRequest {
captured_history: None,
                request_id: operation_id,
                model: model.clone(),
                messages: vec![
                    ModelMessage::System("Synthetic transport acceptance. No tools.".into()),
                    ModelMessage::User { content: vec![ContentPart::Text(
                        "For this synthetic conversation, the fictional sample is called Cedar. This is test data with no research evidence or personal information.".into(),
                    )], source_item_id: None },
                ],
                tools: vec![],
                max_output_tokens: 1024,
                parallel_tool_calls: false,
                reasoning_effort: None,
            },
            max_result_bytes: MAX_BYTES,
        },
    )
    .await?;
    let CompactionOutput::RemoteOpaque(item) = candidate.output else {
        return Err(CompactionFailure::InvalidOutput);
    };
    // Use the actual returned object unchanged. Never manufacture, decode, log,
    // or persist its encrypted content in this provider-only acceptance test.
    let account = candidate
        .credential_scope_id
        .ok_or(CompactionFailure::InvalidOutput)?;
    let continuation = Arc::new(ResponsesContinuation::new(
        Uuid::new_v4(),
        model.clone(),
        MAX_BYTES,
    )?);
    continuation.bind_compaction_sync(account, &model, &item, 0)?;
    let transport = chatgpt_transport(bridge, continuation, capabilities)?;
    let mut stream = transport
        .start(ModelRequest {
            captured_history: None,
            request_id: Uuid::new_v4(),
            model,
            messages: vec![
                ModelMessage::System("Synthetic transport acceptance. Reply briefly.".into()),
                ModelMessage::User {
                    content: vec![ContentPart::Text("Reply with only OK.".into())],
                    source_item_id: None,
                },
            ],
            tools: vec![],
            max_output_tokens: 32,
            parallel_tool_calls: false,
            reasoning_effort: None,
        })
        .await?;
    let mut started = false;
    let mut text = false;
    let mut completed = false;
    while let Some(event) = stream.next().await? {
        match event {
            ModelStreamEvent::Started { .. } => started = true,
            ModelStreamEvent::TextDelta(value)
            | ModelStreamEvent::AssistantTextDelta { text: value, .. } => {
                text |= !value.trim().is_empty()
            }
            ModelStreamEvent::AssistantMessageStarted { .. }
            | ModelStreamEvent::AssistantMessageCompleted { .. }
            | ModelStreamEvent::OutputOrder(_) => (),
            ModelStreamEvent::Completed {
                reason: FinishReason::Stop,
                ..
            } => completed = true,
            ModelStreamEvent::Usage(_) => (),
            _ => return Err(CompactionFailure::InvalidOutput),
        }
    }
    if started && text && completed {
        Ok(())
    } else {
        Err(ModelError::IncompleteStream.into())
    }
}
