//! Real model tool roundtrip with a synthetic, side-effect-free tool and restored checkpoint.
use super::*;
use crate::agent::model::responses_continuation::ResponsesContinuation;
use research_agent_core::model::{ModelResponse, ResponseAssembler, StreamLimits, ToolDefinition};

#[tokio::test]
#[ignore = "two real model calls; requires explicit opt-in and closed development application"]
async fn authenticated_chatgpt_tool_roundtrip_restores_checkpoint() {
    run_roundtrip(None).await;
}

#[tokio::test]
#[ignore = "two real model calls; requires explicit opt-in and closed development application"]
async fn authenticated_chatgpt_medium_tool_roundtrip_restores_checkpoint() {
    run_roundtrip(Some("medium")).await;
}

async fn run_roundtrip(effort: Option<&str>) {
    let _owner = PROFILE_OWNER.lock().await;
    let (bridge, work) = development_bridge();
    let outcome = tokio::time::timeout(
        Duration::from_secs(150),
        roundtrip(bridge.clone(), work.path(), effort),
    )
    .await;
    bridge.stop().await;
    outcome
        .expect("synthetic tool test timed out")
        .expect("synthetic tool roundtrip failed");
    assert_eq!(bridge.next_id.load(Ordering::Acquire), 4);
}

fn transport(
    bridge: CodexAppServerManager,
    state: Arc<ResponsesContinuation>,
    effort: Option<&str>,
) -> Result<ResponsesTransport, ModelError> {
    chatgpt_transport(
        bridge,
        state,
        ModelCapabilities {
            vision: false,
            tools: true,
            parallel_tools: false,
            reasoning_efforts: effort.map(str::to_owned).into_iter().collect(),
        },
    )
}

async fn response(
    transport: &ResponsesTransport,
    request: ModelRequest,
) -> Result<ModelResponse, ModelError> {
    let mut assembler = ResponseAssembler::new(StreamLimits {
        text_bytes: 8192,
        argument_bytes: 1024,
        tool_calls: 1,
    })?
    .with_requested_model(request.model.clone());
    let mut stream = transport.start(request).await?;
    while let Some(event) = stream.next().await? {
        assembler.push(event)?;
    }
    assembler.finish()
}

async fn roundtrip(
    bridge: CodexAppServerManager,
    directory: &std::path::Path,
    effort: Option<&str>,
) -> Result<(), ModelError> {
    let model = ModelIdentity {
        provider_id: "openai".into(),
        model_id: "gpt-5.6-terra".into(),
        revision: None,
    };
    let scope = Uuid::new_v4();
    let state = Arc::new(ResponsesContinuation::new(
        scope,
        model.clone(),
        1024 * 1024,
    )?);
    let mut request = ModelRequest {
captured_history: None,
        request_id: Uuid::new_v4(), model: model.clone(),
        messages: vec![ModelMessage::User { content: vec![ContentPart::Text(
            "Synthetic tool test: call synthetic_lookup exactly once with empty arguments. After its result, reply with only the returned code. Do not guess the code or call again.".into()
        )], source_item_id: None }],
        tools: vec![ToolDefinition {
            namespace: None,
            name: "synthetic_lookup".into(), description: "Returns a synthetic verification code; no files, network or user data.".into(),
            parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
        }],
        max_output_tokens: 256, parallel_tool_calls: false, reasoning_effort: effort.map(str::to_owned),
    };
    let first = response(
        &transport(bridge.clone(), state.clone(), effort)?,
        request.clone(),
    )
    .await?;
    if first.tool_calls.len() != 1 || first.tool_calls[0].name != "synthetic_lookup" {
        return Err(ModelError::InvalidStream);
    }
    let call = first.tool_calls[0]
        .parse()
        .map_err(|_| ModelError::InvalidStream)?;
    if call.arguments != json!({}) {
        return Err(ModelError::InvalidStream);
    }
    // Generated locally after the model's call, so the final answer must use the tool result.
    let code = Uuid::new_v4().simple().to_string();
    request.messages.push(ModelMessage::Assistant {
        phase: None,
        content: if first.text.is_empty() {
            vec![]
        } else {
            vec![ContentPart::Text(first.text)]
        },
        tool_calls: vec![call],
    });
    request.messages.push(ModelMessage::ToolResult {
        call_id: first.tool_calls[0].call_id.clone(),
        content: code.clone(),
        is_error: false,
    });
    let path = directory.join("synthetic-continuation.json");
    std::fs::write(&path, state.encode_checkpoint()?).map_err(|_| ModelError::Unavailable)?;
    drop(state);
    let encoded = std::fs::read_to_string(path).map_err(|_| ModelError::Unavailable)?;
    let restored = Arc::new(ResponsesContinuation::restore(
        scope,
        model,
        1024 * 1024,
        &encoded,
    )?);
    request.request_id = Uuid::new_v4();
    let expected_request = request.request_id;
    let second = response(&transport(bridge, restored.clone(), effort)?, request).await?;
    if !second.tool_calls.is_empty()
        || !second.text.contains(&code)
        || restored.saved_request_id()? != Some(expected_request)
    {
        return Err(ModelError::InvalidStream);
    }
    Ok(())
}
