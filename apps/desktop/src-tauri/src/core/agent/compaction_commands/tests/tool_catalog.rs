use super::*;
use research_agent_core::context::checkpoint::ContextCheckpoint;
use research_agent_core::{model::ToolCall, runner::RunError};

#[tokio::test]
async fn native_manual_catalog_preserves_note_tools_without_callable_handlers() {
    let f = fixture("/compact").await;
    let models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(f._root.path().join("unused-catalog-credentials.json")),
    );
    let profile = execution_profile_for_host(
        &f.record.input.model,
        execution_mode_for_model(&f.record.input.model, None),
        AgentHost::Notes,
    )
    .unwrap();
    let registered = catalog::prepare(
        f.store.clone(),
        f.sessions.clone(),
        f.record.clone(),
        models,
        &f.request.host,
        &profile,
        false,
    )
    .await
    .unwrap();
    let mut expected = crate::agent::note_tools::definitions();
    expected.extend(
        crate::agent::skills::application::tools(f.store.clone(), f.record.clone())
            .await
            .unwrap()
            .definitions(),
    );
    let (definitions, executor) = registered.tools.into_parts();
    for definition in &expected {
        if definition.name != "research_propose_note_rewrite" {
            assert!(definitions.contains(definition));
        }
    }
    assert!(
        !definitions
            .iter()
            .any(|tool| tool.name == "research_propose_note_rewrite")
    );
    assert!(
        definitions
            .iter()
            .any(|tool| tool.name == "research_read_file")
    );
    assert_eq!(
        definitions
            .iter()
            .find(|tool| tool.name == "research_apply_patch")
            .unwrap()
            .parameters["required"],
        serde_json::json!(["patch"])
    );
    assert!(registered.workspace.is_none()); // no frozen shell grant
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "must-not-dispatch".into(),
        name: expected[0].name.clone(),
        arguments: serde_json::json!({}),
    };
    assert!(matches!(
        executor.validate(&call),
        Err(RunError::ToolRejected)
    ));
    assert!(matches!(
        executor.authorize_and_execute(call).await,
        Err(RunError::ToolRejected)
    ));
}

struct PreparedCatalogTransport {
    inner: Arc<Transport>,
    tools: Mutex<Vec<ToolDefinition>>,
    namespace_tools: bool,
}
impl ModelTransport for PreparedCatalogTransport {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn supports_namespace_tools(&self) -> bool {
        self.namespace_tools
    }
    fn validate_compaction_window<'a>(
        &'a self,
        window: &'a ContextCheckpoint,
        request: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        *self.tools.lock().unwrap() = request.tools.clone();
        self.inner.validate_compaction_window(window, request)
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}

#[tokio::test]
async fn native_admission_preserves_host_catalog_through_manual_window_validation() {
    let f = fixture("/compact").await;
    let inner = transport(&f, false);
    let captured = Arc::new(PreparedCatalogTransport {
        inner: inner.clone(),
        tools: Mutex::new(vec![]),
        namespace_tools: false,
    });
    let models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(f._root.path().join("unused-admission-credentials.json")),
    );
    let admission = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |record, profile| {
            let store = f.store.clone();
            let sessions = f.sessions.clone();
            let host = f.request.host.clone();
            let captured = captured.clone();
            async move {
                let registered =
                    catalog::prepare(store, sessions, record, models, &host, &profile, false)
                        .await?;
                Ok((captured as Arc<dyn ModelTransport>, None, registered.tools))
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(wait(admission).await, TaskCompletion::Completed);
    let tools = captured.tools.lock().unwrap();
    for definition in crate::agent::note_tools::definitions() {
        if definition.name != "research_propose_note_rewrite" {
            assert!(tools.iter().any(|actual| actual == &definition));
        }
    }
    assert!(tools.iter().any(|tool| tool.name == "research_apply_patch"));
    assert!(
        tools
            .iter()
            .all(|actual| actual.name != "research_spawn_subtask")
    );
    assert!(
        tools
            .iter()
            .any(|actual| actual.name == "research_read_source")
    );
    assert!(inner.requests.lock().unwrap()[0].tools.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn native_manual_catalog_freezes_approved_mcp_for_both_provider_modes() {
    use crate::agent::mcp_commands::{InstallRequest, install_mcp_runtime_from_config};
    let f = fixture("/compact").await;
    let root = f.store.agent_configuration_directory();
    std::fs::create_dir_all(root.join("agent")).unwrap();
    let script = r#"while IFS= read line; do case "$line" in *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;; *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"find-doc","description":"find","inputSchema":{"type":"object"}}]}}' ;; *) exit 1 ;; esac; done"#;
    let config = format!(
        "schema_version=1\n[servers.literature]\nenvironment_id='fixture'\nrevision='1'\nprogram='/bin/sh'\ndirectory={}\narguments=['-c',{}]\n",
        serde_json::to_string(f._root.path().to_str().unwrap()).unwrap(),
        serde_json::to_string(script).unwrap()
    );
    std::fs::write(root.join("agent/mcp.toml"), config).unwrap();
    let credentials = JsonCredentialStore::from_path(f._root.path().join("unused-mcp.json"));
    install_mcp_runtime_from_config(
        &f.store,
        &credentials,
        &f.sessions,
        InstallRequest {
            approved_server_ids: vec!["literature".into()],
        },
    )
    .await
    .unwrap();
    let models = AgentModelResolver::from_app_store(&f.store, credentials);
    let profile = execution_profile_for_host(
        &f.record.input.model,
        execution_mode_for_model(&f.record.input.model, None),
        AgentHost::Notes,
    )
    .unwrap();
    for namespace_tools in [false, true] {
        let registered = catalog::prepare(
            f.store.clone(),
            f.sessions.clone(),
            f.record.clone(),
            models.clone(),
            &f.request.host,
            &profile,
            namespace_tools,
        )
        .await
        .unwrap();
        let (definitions, executor) = registered.tools.into_parts();
        let mcp = definitions
            .iter()
            .find(|tool| {
                if namespace_tools {
                    tool.namespace.as_deref() == Some("mcp__literature") && tool.name == "find_doc"
                } else {
                    tool.namespace.is_none()
                        && tool.name.contains("literature")
                        && tool.name.contains("find")
                }
            })
            .expect("approved MCP definition in manual compaction catalog");
        let call = ToolCall {
            namespace: mcp.namespace.clone(),
            name: mcp.name.clone(),
            call_id: "compact-must-not-dispatch".into(),
            raw_arguments_json: None,
            arguments: serde_json::json!({}),
        };
        assert!(matches!(
            executor.validate(&call),
            Err(RunError::ToolRejected)
        ));
        assert!(matches!(
            executor.authorize_and_execute(call).await,
            Err(RunError::ToolRejected)
        ));
    }
    let inner = transport(&f, false);
    let captured = Arc::new(PreparedCatalogTransport {
        inner: inner.clone(),
        tools: Mutex::new(vec![]),
        namespace_tools: true,
    });
    let admission = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |record, profile| {
            let captured = captured.clone();
            let store = f.store.clone();
            let sessions = f.sessions.clone();
            let host = f.request.host.clone();
            async move {
                let registered = catalog::prepare(
                    store,
                    sessions,
                    record,
                    models,
                    &host,
                    &profile,
                    captured.supports_namespace_tools(),
                )
                .await?;
                Ok((captured as Arc<dyn ModelTransport>, None, registered.tools))
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(wait(admission).await, TaskCompletion::Completed);
    assert!(
        captured
            .tools
            .lock()
            .unwrap()
            .iter()
            .any(|tool| tool.namespace.as_deref() == Some("mcp__literature")
                && tool.name == "find_doc")
    );
    assert!(inner.requests.lock().unwrap()[0].tools.is_empty());
}
