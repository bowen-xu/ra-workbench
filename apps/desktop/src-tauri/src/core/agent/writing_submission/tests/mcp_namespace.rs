use super::*;
use crate::agent::mcp_runtime::McpToolProvider;
use research_agent_core::{
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolNameResolution, ToolOutput},
};
use std::{collections::HashSet, sync::Arc};

struct Provider;
impl McpToolProvider for Provider {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            namespace: Some("mcp__fixture".into()),
            name: "research_spawn_subtask".into(),
            description: "fixture".into(),
            parameters: serde_json::json!({"type":"object"}),
        }]
    }
    fn approved_servers(&self) -> HashSet<String> {
        HashSet::from(["fixture".into()])
    }
    fn namespaced_view(self: Arc<Self>) -> Arc<dyn McpToolProvider> {
        self
    }
}
impl ToolExecutor for Provider {
    fn resolve_qualified_name(&self, namespace: Option<&str>, name: &str) -> ToolNameResolution {
        if namespace == Some("mcp__fixture") && name == "research_spawn_subtask" {
            ToolNameResolution::Registered
        } else {
            ToolNameResolution::NotFound
        }
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if self.resolve_qualified_name(call.namespace.as_deref(), &call.name)
            == ToolNameResolution::Registered
        {
            Ok(())
        } else {
            Err(RunError::ToolNotFound)
        }
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            self.validate(&call)?;
            Ok(ToolOutput {
                content: "MCP owner".into(),
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn namespaced_mcp_does_not_route_to_same_named_writing_tool() {
    let f = fixture(true);
    let prepared = WritingSubmissionService::from_app_store(&f.store)
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    let sessions =
        Arc::new(crate::agent::service::AgentSessionService::from_app_store(&f.store).unwrap());
    let tools = super::super::tools::for_submission_with_mcp(
        f.store.clone(),
        prepared.record,
        sessions,
        Some(Arc::new(Provider)),
    )
    .await
    .unwrap();
    let call = ToolCall {
        call_id: "call".into(),
        namespace: Some("mcp__fixture".into()),
        name: "research_spawn_subtask".into(),
        arguments: serde_json::json!({}),
        raw_arguments_json: None,
    };
    assert_eq!(
        tools.resolve_qualified_name(call.namespace.as_deref(), &call.name),
        ToolNameResolution::Registered
    );
    tools.validate(&call).unwrap();
    assert_eq!(
        tools
            .authorize_and_execute(call.clone())
            .await
            .unwrap()
            .content,
        "MCP owner"
    );
    let mut foreign = call;
    foreign.namespace = Some("foreign".into());
    assert!(tools.validate(&foreign).is_err());
    assert!(tools.authorize_and_execute(foreign).await.is_err());
}
