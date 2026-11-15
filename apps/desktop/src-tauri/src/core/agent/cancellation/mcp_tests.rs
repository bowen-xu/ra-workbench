use super::*;
use crate::agent::{
    mcp_stdio::StdioClient,
    mcp_tools::{Client, McpTools},
    process::ProcessSpec,
};
use research_agent_core::mcp::ToolSpec;
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf, time::Duration};

#[tokio::test]
async fn canceling_mcp_with_backpressured_stdin_waits_for_its_configured_owner_deadline() {
    let client = StdioClient::spawn(ProcessSpec {
        program: PathBuf::from("/bin/sh"),
        // The synthetic server never reads a request or accesses an account.
        arguments: vec![OsString::from("-c"), OsString::from("kill -STOP $$")],
        directory: std::env::temp_dir(),
        environment: BTreeMap::new(),
    })
    .unwrap();
    let tools = McpTools::new(
        "fixture".into(),
        "r1".into(),
        Client::Stdio(client),
        vec![ToolSpec {
            name: "search".into(),
            description: String::new(),
            input_schema: serde_json::json!({"type":"object"}),
        }],
        true,
        Duration::from_millis(100),
    )
    .unwrap();
    let cancellation = AgentCancellation::default();
    let wrapper = CancellableTools {
        inner: &tools,
        cancellation: cancellation.clone(),
    };
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "mcp-blocked-send".into(),
        name: "fixture__search".into(),
        arguments: serde_json::json!({"text":"x".repeat(1024 * 1024)}),
    };
    assert!(!wrapper.cancellation_safe_to_drop(&call));
    let mut execution = wrapper.authorize_and_execute(call.clone());
    assert!(
        std::future::poll_fn(|cx| std::task::Poll::Ready(execution.as_mut().poll(cx).is_pending()))
            .await
    );
    cancellation.cancel();
    // Cancellation joins this external owner. Its existing request deadline
    // must cover a blocked write, poison the client and prevent late reuse.
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), execution)
            .await
            .unwrap(),
        Err(RunError::ToolOutcomeUnknown)
    ));
    assert!(matches!(
        tools.authorize_and_execute(call).await,
        Err(RunError::ToolUnavailable)
    ));
}
