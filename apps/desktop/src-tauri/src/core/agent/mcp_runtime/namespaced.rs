//! Namespace catalog view over the same discovered clients and approvals.
use super::*;
use research_agent_core::{
    mcp::{McpModelName, McpRawName, normalize_model_names},
    model::ToolDefinition,
};

/// A frozen model-visible catalog and its original MCP routing identities.
pub struct NamespacedMcpRuntime {
    runtime: Arc<McpRuntime>,
    allowed_servers: HashSet<String>,
    catalog: Vec<McpModelName>,
}

impl McpRuntime {
    /// Build one deterministic catalog from all discovered servers before
    /// restricting the view. This keeps identities stable across child scopes.
    pub fn namespaced(self: &Arc<Self>, allowed_servers: HashSet<String>) -> NamespacedMcpRuntime {
        let raw = self.tools.iter().flat_map(|server| {
            server.tool_specs().iter().map(move |tool| McpRawName {
                server: server.server_id().to_owned(),
                namespace: server.server_id().to_owned(),
                callable_name: tool.name.clone(),
                tool_name: tool.name.clone(),
                connector_id: None,
            })
        });
        NamespacedMcpRuntime {
            runtime: Arc::clone(self),
            allowed_servers,
            catalog: normalize_model_names(raw, true, &[]),
        }
    }
}

impl NamespacedMcpRuntime {
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.catalog
            .iter()
            .filter(|entry| self.allowed_servers.contains(&entry.raw.server))
            .map(|entry| {
                let server = self
                    .runtime
                    .tools
                    .iter()
                    .find(|server| server.server_id() == entry.raw.server)
                    .expect("catalog server came from this runtime");
                let tool = server
                    .tool_specs()
                    .iter()
                    .find(|tool| tool.name == entry.raw.tool_name)
                    .expect("catalog tool came from this runtime");
                ToolDefinition {
                    namespace: Some(entry.namespace.clone()),
                    name: entry.name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                }
            })
            .collect()
    }

    fn entry(&self, namespace: Option<&str>, name: &str) -> Option<&McpModelName> {
        self.catalog
            .iter()
            .find(|entry| Some(entry.namespace.as_str()) == namespace && entry.name == name)
    }

    fn original<'a>(&'a self, call: &ToolCall) -> Result<(&'a McpTools, ToolCall), RunError> {
        let entry = self
            .entry(call.namespace.as_deref(), &call.name)
            .ok_or(RunError::ToolNotFound)?;
        if !self.allowed_servers.contains(&entry.raw.server) {
            return Err(RunError::ToolRejected);
        }
        let server = self
            .runtime
            .tools
            .iter()
            .find(|server| server.server_id() == entry.raw.server)
            .ok_or(RunError::ToolNotFound)?;
        let mut original = call.clone();
        original.namespace = None;
        original.name = format!("{}__{}", entry.raw.server, entry.raw.tool_name);
        Ok((server, original))
    }
}

impl McpToolProvider for NamespacedMcpRuntime {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.definitions()
    }
    fn approved_servers(&self) -> HashSet<String> {
        self.allowed_servers.clone()
    }
    fn namespaced_view(self: Arc<Self>) -> Arc<dyn McpToolProvider> {
        self
    }
}

impl ToolExecutor for NamespacedMcpRuntime {
    fn resolve_qualified_name(&self, namespace: Option<&str>, name: &str) -> ToolNameResolution {
        if self.entry(namespace, name).is_some() {
            ToolNameResolution::Registered
        } else {
            ToolNameResolution::NotFound
        }
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        let (server, original) = self.original(call)?;
        server.validate(&original)
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        match self.original(&call) {
            Ok((server, original)) => server.authorize_and_execute(original),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        match self.original(&call) {
            Ok((server, original)) => server.result_committed(original),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
}

#[cfg(test)]
#[path = "namespaced/tests.rs"]
mod tests;
