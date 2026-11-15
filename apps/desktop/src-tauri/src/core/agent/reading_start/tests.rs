use super::*;

#[test]
fn host_definitions_only_contain_reading_extensions() {
    let definitions = reading_host_tool_definitions();
    assert!(definitions.is_empty());
    assert!(definitions.iter().all(|tool| !matches!(
        tool.name.as_str(),
        "research_search_sources" | "research_list_sources" | "research_read_source"
    )));
}

#[test]
fn disabled_subtask_has_no_reading_registration_or_handler() {
    use research_agent_core::model::ToolCall;
    let registered = compose_host_tools(
        Arc::new(RejectingToolExecutor),
        Arc::new(RejectingToolExecutor),
        vec![],
        Arc::new(RejectingToolExecutor),
        vec![],
    );
    let (definitions, executor) = registered.into_parts();
    let call = ToolCall {
        call_id: "disabled".into(),
        name: "research_spawn_subtask".into(),
        namespace: None,
        arguments: serde_json::json!({"prompt":"test"}),
        raw_arguments_json: None,
    };
    assert!(definitions.iter().all(|tool| tool.name != call.name));
    assert!(executor.validate(&call).is_err());
    assert!(executor.child_creation_intent(&call).is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn reading_catalog_routes_mcp_through_the_registered_owner() {
    use crate::agent::{
        mcp_config::{ConfiguredServer, Transport},
        mcp_runtime::McpRuntime,
    };
    use research_agent_core::{
        model::ToolCall,
        runner::{RunError, ToolNameResolution},
        tool_registry::ToolRegistry,
    };
    use std::{
        collections::{BTreeMap, HashSet},
        ffi::OsString,
        path::PathBuf,
        time::Duration,
    };
    let script = r#"id=3; while IFS= read line; do case "$line" in
      *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
      *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"search","inputSchema":{"type":"object"}}]}}' ;;
      *'"name":"search"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"reading MCP evidence"}]}}\n' "$id"; id=$((id+1)) ;;
      *) exit 1 ;;
    esac; done"#;
    let server = ConfiguredServer {
        server_id: "literature".into(),
        environment_id: "fixture".into(),
        revision: "r1".into(),
        transport: Transport::Stdio {
            program: PathBuf::from("/bin/sh"),
            arguments: vec![OsString::from("-c"), OsString::from(script)],
            directory: std::env::temp_dir(),
        },
        environment: BTreeMap::new(),
        credential_handle: None,
        enabled: true,
    };
    let runtime = Arc::new(
        McpRuntime::build(
            vec![server],
            &HashSet::from(["literature".into()]),
            Duration::from_secs(3),
        )
        .await
        .unwrap(),
    );
    let registered = compose_host_tools(
        Arc::new(RejectingToolExecutor),
        Arc::new(RejectingToolExecutor),
        vec![],
        runtime.clone(),
        runtime.definitions(),
    );
    let registry = ToolRegistry::from_registrations([registered.borrowed()]).unwrap();
    let call = ToolCall {
        call_id: "call".into(),
        name: "literature__search".into(),
        namespace: None,
        arguments: serde_json::json!({}),
        raw_arguments_json: None,
    };
    assert_eq!(
        registry.resolve_name(&call.name),
        ToolNameResolution::Registered
    );
    registry.validate(&call).unwrap();
    let result = registry.authorize_and_execute(call.clone()).await.unwrap();
    assert!(result.content.contains("reading MCP evidence"));
    registry.result_committed(call).await.unwrap();
    let missing = ToolCall {
        call_id: "missing".into(),
        name: "literature__missing".into(),
        namespace: None,
        arguments: serde_json::json!({}),
        raw_arguments_json: None,
    };
    assert!(matches!(
        registry.authorize_and_execute(missing).await,
        Err(RunError::ToolNotFound)
    ));
    let namespaced = Arc::new(runtime.namespaced(HashSet::from(["literature".into()])));
    let registered = compose_host_tools(
        Arc::new(RejectingToolExecutor),
        Arc::new(RejectingToolExecutor),
        vec![],
        namespaced.clone(),
        namespaced.definitions(),
    );
    let registry = ToolRegistry::from_registrations([registered.borrowed()]).unwrap();
    let call = ToolCall {
        call_id: "qualified".into(),
        name: "search".into(),
        namespace: Some("mcp__literature".into()),
        arguments: serde_json::json!({}),
        raw_arguments_json: None,
    };
    assert_eq!(
        registry.resolve_qualified_name(call.namespace.as_deref(), &call.name),
        ToolNameResolution::Registered
    );
    registry.validate(&call).unwrap();
    assert!(
        registry
            .authorize_and_execute(call.clone())
            .await
            .unwrap()
            .content
            .contains("reading MCP evidence")
    );
    registry.result_committed(call).await.unwrap();
}
