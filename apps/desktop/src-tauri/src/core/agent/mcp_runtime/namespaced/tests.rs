use super::*;
use research_agent_core::tool_registry::{ToolRegistration, ToolRegistry};
use serde_json::json;
use std::{ffi::OsString, path::PathBuf};

fn server(id: &str) -> ConfiguredServer {
    let script = r#"while IFS= read line; do case "$line" in
      *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
      *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"get-strudel-guide","description":"guide","inputSchema":{"type":"object"}}]}}' ;;
      *'"name":"get-strudel-guide"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"raw MCP name used"}]}}' ;;
      *) exit 1 ;;
    esac; done"#;
    ConfiguredServer {
        server_id: id.into(),
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
    }
}

#[tokio::test]
async fn normalized_namespace_routes_to_original_stdio_name_and_scope() {
    let runtime = Arc::new(
        McpRuntime::build(
            vec![server("music-studio"), server("music_studio")],
            &HashSet::from(["music-studio".into(), "music_studio".into()]),
            Duration::from_secs(3),
        )
        .await
        .unwrap(),
    );
    let all = runtime.namespaced(HashSet::from([
        "music-studio".into(),
        "music_studio".into(),
    ]));
    let definitions = all.definitions();
    assert_eq!(definitions.len(), 2);
    assert_ne!(definitions[0].namespace, definitions[1].namespace);
    assert!(
        definitions
            .iter()
            .all(|definition| definition.name == "get_strudel_guide")
    );
    let selected = &all.catalog[0];
    let one_server = runtime.namespaced(HashSet::from([selected.raw.server.clone()]));
    let one_definition = one_server.definitions();
    assert_eq!(one_definition.len(), 1);
    assert_eq!(
        one_definition[0].namespace.as_deref(),
        Some(selected.namespace.as_str())
    );
    assert_eq!(one_definition[0].name, selected.name);
    let call = ToolCall {
        call_id: "call".into(),
        namespace: Some(selected.namespace.clone()),
        name: selected.name.clone(),
        arguments: json!({}),
        raw_arguments_json: Some("{}".into()),
    };
    let restricted = runtime.namespaced(HashSet::new());
    assert!(restricted.definitions().is_empty());
    assert_eq!(
        restricted.resolve_qualified_name(call.namespace.as_deref(), &call.name),
        ToolNameResolution::Registered
    );
    assert!(matches!(
        restricted.validate(&call),
        Err(RunError::ToolRejected)
    ));
    assert!(matches!(
        restricted.authorize_and_execute(call.clone()).await,
        Err(RunError::ToolRejected)
    ));
    let mut foreign = call.clone();
    foreign.namespace = Some("foreign".into());
    assert!(matches!(
        all.authorize_and_execute(foreign).await,
        Err(RunError::ToolNotFound)
    ));
    let registry =
        ToolRegistry::from_registrations([ToolRegistration::new(definitions, &all)]).unwrap();
    registry.validate(&call).unwrap();
    let result = registry.authorize_and_execute(call.clone()).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("raw MCP name used"));
    registry.result_committed(call).await.unwrap();
}
