//! MCP tool executor built on the host-owned stdio client.
//!
//! The process and approval inputs are constructed by the host. A model can
//! only select a tool from the frozen, namespaced discovery result.

use super::mcp_http;
use super::mcp_stdio::StdioClient;
use research_agent_core::{
    extensions::ExtensionRegistry,
    mcp::{self, Request, Response, ToolSpec},
    model::{ToolCall, ToolDefinition},
    runner::{RunError, RunFuture, ToolExecutor, ToolNameResolution, ToolOutput},
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::Mutex;

const MAX_TOOL_NAME_BYTES: usize = 512;

pub struct McpTools {
    server_id: String,
    revision: String,
    client: Arc<Mutex<Client>>,
    tools: Vec<ToolSpec>,
    approved: std::sync::atomic::AtomicBool,
    next_request_id: AtomicU64,
    timeout: Duration,
}

pub(crate) enum Client {
    Stdio(StdioClient),
    Http(mcp_http::Client),
}

impl Client {
    async fn request(&mut self, request: &Request, timeout: Duration) -> Result<Response, ()> {
        match self {
            Self::Stdio(client) => client.request(request, timeout).await.map_err(|_| ()),
            Self::Http(client) => client.request(request, timeout).await.map_err(|_| ()),
        }
    }
}

impl McpTools {
    pub(crate) async fn thread_hint(
        &self,
        conversation: uuid::Uuid,
    ) -> Result<Option<String>, RunError> {
        if self.server_id != "notes"
            || !self.is_approved()
            || !self.tools.iter().any(|tool| tool.name == "thread_hint")
        {
            return Err(RunError::ToolRejected);
        }
        let response = self
            .call_request(
                json!({"name":"thread_hint", "_meta":{"threadId":conversation.to_string()}}),
            )
            .await?;
        mcp::decode_thread_hint(response).map_err(|_| RunError::ToolUnavailable)
    }

    async fn call_request(&self, params: serde_json::Value) -> Result<Response, RunError> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = Request {
            jsonrpc: "2.0".into(),
            id,
            method: "tools/call".into(),
            params,
        };
        // Validate the exact frame before acquiring a wire owner. A rejected
        // local request is known not to have caused an external effect.
        mcp::encode_request(&request).map_err(|_| {
            RunError::ToolArguments("MCP request exceeds the supported wire contract".into())
        })?;
        // One configured deadline covers owner admission and the complete wire
        // request. Timing out in the queue has not started or poisoned a client.
        let mut client = tokio::time::timeout_at(deadline, self.client.lock())
            .await
            .map_err(|_| RunError::ToolUnavailable)?;
        if tokio::time::Instant::now() >= deadline {
            return Err(RunError::ToolUnavailable);
        }
        if matches!(&*client, Client::Stdio(client) if client.is_failed()) {
            return Err(RunError::ToolUnavailable);
        }
        client
            .request(
                &request,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            )
            .await
            // The request may have reached the server. A transport timeout,
            // disconnect or invalid reply is not permission to repeat effects.
            .map_err(|_| RunError::ToolOutcomeUnknown)
    }
    pub fn new_authorized(
        server_id: String,
        revision: String,
        client: StdioClient,
        tools: Vec<ToolSpec>,
        registry: &ExtensionRegistry,
        approved_servers: &HashSet<String>,
        timeout: Duration,
    ) -> Result<Self, RunError> {
        let result = Self::new(
            server_id,
            revision,
            Client::Stdio(client),
            tools,
            false,
            timeout,
        )?;
        result.authorize_with_registry(registry, approved_servers)?;
        Ok(result)
    }

    pub async fn discover(
        server_id: String,
        revision: String,
        client: StdioClient,
        approved: bool,
        timeout: Duration,
    ) -> Result<Self, RunError> {
        Self::discover_client(
            server_id,
            revision,
            Client::Stdio(client),
            approved,
            timeout,
        )
        .await
    }

    pub(crate) async fn discover_client(
        server_id: String,
        revision: String,
        mut client: Client,
        approved: bool,
        timeout: Duration,
    ) -> Result<Self, RunError> {
        let _initialize_response = client
            .request(
                &Request {
                    jsonrpc: "2.0".into(),
                    id: 1,
                    method: "initialize".into(),
                    params: json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": {"name": "research-assistant", "version": env!("CARGO_PKG_VERSION")}
                    }),
                },
                timeout,
            )
            .await
            .map_err(|_| RunError::ToolUnavailable)?;
        let response = client
            .request(
                &Request {
                    jsonrpc: "2.0".into(),
                    id: 2,
                    method: "tools/list".into(),
                    params: json!({}),
                },
                timeout,
            )
            .await
            .map_err(|_| RunError::ToolUnavailable)?;
        let tools = mcp::decode_tools_list(response).map_err(|_| RunError::ToolUnavailable)?;
        let result = Self::new(server_id, revision, client, tools, approved, timeout)?;

        result.next_request_id.store(3, Ordering::Relaxed);
        Ok(result)
    }

    pub(crate) fn new(
        server_id: String,
        revision: String,
        client: Client,
        tools: Vec<ToolSpec>,
        approved: bool,
        timeout: Duration,
    ) -> Result<Self, RunError> {
        if server_id.trim().is_empty() || revision.trim().is_empty() || tools.is_empty() {
            return Err(RunError::ToolRejected);
        }
        if tools
            .iter()
            .any(|tool| tool.name.len() > MAX_TOOL_NAME_BYTES)
        {
            return Err(RunError::ToolRejected);
        }
        Ok(Self {
            server_id,
            revision,
            client: Arc::new(Mutex::new(client)),
            tools,
            approved: std::sync::atomic::AtomicBool::new(approved),
            next_request_id: AtomicU64::new(1),
            timeout,
        })
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|tool| ToolDefinition {
                namespace: None,
                name: self.qualified(&tool.name),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            })
            .collect()
    }

    pub fn handles(&self, name: &str) -> bool {
        self.tools
            .iter()
            .any(|tool| self.qualified(&tool.name) == name)
    }

    pub fn tool_specs(&self) -> &[ToolSpec] {
        &self.tools
    }

    pub fn authorize_with_registry(
        &self,
        registry: &ExtensionRegistry,
        approved_servers: &HashSet<String>,
    ) -> Result<(), RunError> {
        for tool in &self.tools {
            let qualified = self.qualified(&tool.name);
            registry
                .authorize(
                    &self.server_id,
                    &self.revision,
                    &qualified,
                    approved_servers,
                )
                .map_err(|_| RunError::ToolRejected)?;
        }
        self.approved.store(true, Ordering::Release);
        Ok(())
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn is_approved(&self) -> bool {
        self.approved.load(Ordering::Acquire)
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    fn qualified(&self, name: &str) -> String {
        format!("{}__{}", self.server_id, name)
    }

    fn selected(&self, call: &ToolCall) -> Result<String, RunError> {
        if !self.approved.load(Ordering::Acquire) {
            return Err(RunError::ToolRejected);
        }
        let prefix = format!("{}__", self.server_id);
        let name = call
            .name
            .strip_prefix(&prefix)
            .ok_or(RunError::ToolRejected)?;
        if self.tools.iter().any(|tool| tool.name == name) {
            Ok(name.to_owned())
        } else {
            Err(RunError::ToolRejected)
        }
    }
}

impl ToolExecutor for McpTools {
    fn resolve_name(&self, name: &str) -> ToolNameResolution {
        if self.handles(name) {
            ToolNameResolution::Registered
        } else {
            ToolNameResolution::NotFound
        }
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        self.selected(call).map(|_| ())
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            let name = self.selected(&call)?;
            let response = self
                .call_request(json!({"name": name, "arguments": call.arguments}))
                .await?;
            let result =
                mcp::decode_tool_result(response).map_err(|_| RunError::ToolOutcomeUnknown)?;
            Ok(ToolOutput {
                content: result.content,
                is_error: result.is_error,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::extensions::{ExtensionDescriptor, ExtensionTool};
    use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

    fn client() -> StdioClient {
        StdioClient::spawn(crate::agent::process::ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            arguments: vec![OsString::from("-c"), OsString::from("exit 0")],
            directory: std::env::temp_dir(),
            environment: BTreeMap::new(),
        })
        .unwrap()
    }

    fn fixture_tools(client: StdioClient, timeout: Duration) -> McpTools {
        McpTools::new(
            "fixture".into(),
            "r1".into(),
            Client::Stdio(client),
            vec![ToolSpec {
                name: "search".into(),
                description: String::new(),
                input_schema: json!({"type":"object"}),
            }],
            true,
            timeout,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn name_lookup_is_pure_and_known_handler_stays_known_when_approval_is_revoked() {
        let tools = fixture_tools(client(), Duration::from_secs(1));
        let _owner = tools.client.lock().await;
        assert_eq!(
            tools.resolve_name("fixture__search"),
            ToolNameResolution::Registered
        );
        assert_eq!(
            tools.resolve_name("fixture__not_search"),
            ToolNameResolution::NotFound
        );
        tools.approved.store(false, Ordering::Release);
        assert_eq!(
            tools.resolve_name("fixture__search"),
            ToolNameResolution::Registered
        );
        assert!(matches!(
            tools.validate(&ToolCall {
                namespace: None,
                call_id: "denied".into(),
                name: "fixture__search".into(),
                arguments: json!({}),
                raw_arguments_json: None
            }),
            Err(RunError::ToolRejected)
        ));
    }

    #[tokio::test]
    async fn configured_deadline_includes_waiting_for_the_mcp_owner() {
        let tools = fixture_tools(client(), Duration::from_millis(20));
        let _owner = tools.client.lock().await;
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            tools.call_request(json!({"name":"search","arguments":{}})),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(RunError::ToolUnavailable)));
    }

    #[tokio::test]
    async fn invalid_reply_after_dispatch_is_an_unknown_outcome_not_retry_feedback() {
        let client = StdioClient::spawn(crate::agent::process::ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            arguments: vec![OsString::from("-c"), OsString::from(
                "IFS= read line; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"unexpected\":true}}'"
            )],
            directory: std::env::temp_dir(), environment: BTreeMap::new(),
        }).unwrap();
        let tools = fixture_tools(client, Duration::from_secs(2));
        assert!(matches!(
            tools
                .authorize_and_execute(ToolCall {
                    namespace: None,
                    raw_arguments_json: None,
                    call_id: "unknown-result".into(),
                    name: "fixture__search".into(),
                    arguments: json!({}),
                })
                .await,
            Err(RunError::ToolOutcomeUnknown)
        ));
    }

    #[tokio::test]
    async fn oversized_local_frame_is_rejected_before_dispatch() {
        let tools = fixture_tools(client(), Duration::from_secs(1));
        assert!(matches!(
            tools
                .authorize_and_execute(ToolCall {
                    namespace: None,
                    raw_arguments_json: None,
                    call_id: "frame-too-large".into(),
                    name: "fixture__search".into(),
                    arguments: json!({"text":"x".repeat(2 * 1024 * 1024)}),
                })
                .await,
            Err(RunError::ToolArguments(_))
        ));
        let client = tools.client.lock().await;
        assert!(matches!(&*client, Client::Stdio(client) if !client.is_failed()));
    }

    #[tokio::test]
    async fn registry_revision_and_turn_approval_gate_adapter_creation() {
        let mut registry = ExtensionRegistry::default();
        registry
            .register(ExtensionDescriptor {
                environment_id: "project".into(),
                server_id: "literature".into(),
                revision: "r1".into(),
                tools: vec![ExtensionTool {
                    name: "search".into(),
                    description: "search".into(),
                    input_schema: json!({"type":"object"}),
                }],
            })
            .unwrap();
        let tools = vec![ToolSpec {
            name: "search".into(),
            description: "search".into(),
            input_schema: json!({"type":"object"}),
        }];
        let approved = HashSet::from(["literature".to_string()]);
        assert!(
            McpTools::new_authorized(
                "literature".into(),
                "old".into(),
                client(),
                tools.clone(),
                &registry,
                &approved,
                Duration::from_secs(1),
            )
            .is_err()
        );
        let adapter = McpTools::new_authorized(
            "literature".into(),
            "r1".into(),
            client(),
            tools,
            &registry,
            &approved,
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(adapter.handles("literature__search"));
    }
    #[tokio::test]
    async fn unapproved_wrong_server_or_missing_tool_cannot_read_thread_hint() {
        for (server, tool, approved) in [
            ("notes", "thread_hint", false),
            ("other", "thread_hint", true),
            ("notes", "other", true),
        ] {
            let adapter = McpTools::new(
                server.into(),
                "r1".into(),
                Client::Stdio(client()),
                vec![ToolSpec {
                    name: tool.into(),
                    description: String::new(),
                    input_schema: json!({"type":"object"}),
                }],
                approved,
                Duration::from_secs(1),
            )
            .unwrap();
            assert!(matches!(
                adapter.thread_hint(uuid::Uuid::new_v4()).await,
                Err(RunError::ToolRejected)
            ));
            assert_eq!(adapter.next_request_id.load(Ordering::Relaxed), 1);
        }
    }
}
