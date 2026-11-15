//! Host-owned MCP runtime assembly.
//!
//! This is the narrow composition point between trusted application config,
//! MCP discovery, the core extension registry and per-turn approval. It does
//! not expose a way for model input to create or reconfigure a server.

use super::{
    mcp_config::{ConfiguredServer, Transport},
    mcp_http,
    mcp_stdio::StdioClient,
    mcp_tools::{Client, McpTools},
};
#[path = "mcp_runtime/namespaced.rs"]
mod namespaced;
#[path = "mcp_runtime/thread_hint.rs"]
mod thread_hint;
pub use namespaced::NamespacedMcpRuntime;
use research_agent_core::extensions::{ExtensionDescriptor, ExtensionRegistry, ExtensionTool};
use research_agent_core::{
    model::ToolCall,
    runner::{RunError, RunFuture, ToolExecutor, ToolNameResolution, ToolOutput},
};
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    sync::Arc,
    time::Duration,
};

/// Resolves an opaque, host-owned credential handle immediately before a
/// server process is spawned. Implementations must never persist or log the
/// returned values; the runtime only puts them into the child environment.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("credential handle is unavailable")]
    Unavailable,
}

pub trait CredentialResolver: Send + Sync {
    fn resolve(&self, handle: &str) -> Result<BTreeMap<OsString, OsString>, CredentialError>;
}

struct NoCredentials;
impl CredentialResolver for NoCredentials {
    fn resolve(&self, _handle: &str) -> Result<BTreeMap<OsString, OsString>, CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

/// Adapter over the application's existing private credential store. It
/// returns an explicit environment entry only for a valid `mcp:` handle.
pub struct JsonMcpCredentialResolver {
    store: crate::ai::JsonCredentialStore,
}

impl JsonMcpCredentialResolver {
    pub(crate) fn new(store: crate::ai::JsonCredentialStore) -> Self {
        Self { store }
    }
}

impl CredentialResolver for JsonMcpCredentialResolver {
    fn resolve(&self, handle: &str) -> Result<BTreeMap<OsString, OsString>, CredentialError> {
        let secret = self
            .store
            .resolve_mcp(handle)
            .map_err(|_| CredentialError::Unavailable)?;
        let mut values = BTreeMap::new();
        values.insert(
            OsString::from("RESEARCH_ASSISTANT_MCP_SECRET"),
            OsString::from(secret.expose()),
        );
        Ok(values)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("MCP server process could not start")]
    Process,
    #[error("MCP server discovery failed")]
    Discovery,
    #[error("MCP server registration was rejected")]
    Registration,
    #[error("MCP server authorization was rejected")]
    Authorization,
}

pub struct McpRuntime {
    registry: ExtensionRegistry,
    tools: Vec<McpTools>,
    identities: Vec<(String, String, String)>,
}

/// A host-facing MCP catalog.  The catalog owns both provider definitions and
/// execution, so a scoped view cannot accidentally advertise one set of tools
/// while dispatching another set.
pub(crate) trait McpToolProvider: ToolExecutor {
    fn definitions(&self) -> Vec<research_agent_core::model::ToolDefinition>;
    fn approved_servers(&self) -> HashSet<String>;
    fn namespaced_view(self: Arc<Self>) -> Arc<dyn McpToolProvider>;
}

/// A narrow view over an already discovered runtime.  It deliberately reuses
/// the parent's clients and approval state; only the server allowlist changes.
/// Child tasks therefore do not create a second MCP transport or discovery
/// path.
pub(crate) struct ScopedMcpRuntime {
    runtime: Arc<McpRuntime>,
    allowed_servers: HashSet<String>,
}

impl McpRuntime {
    pub(crate) fn scoped(
        self: &Arc<Self>,
        allowed_servers: HashSet<String>,
    ) -> Arc<dyn McpToolProvider> {
        Arc::new(ScopedMcpRuntime {
            runtime: Arc::clone(self),
            allowed_servers,
        })
    }
}

fn register_adapter(
    registry: &mut ExtensionRegistry,
    tools: &mut Vec<McpTools>,
    server: ConfiguredServer,
    adapter: McpTools,
    approved_servers: &HashSet<String>,
) -> Result<(), Error> {
    let descriptor = ExtensionDescriptor {
        environment_id: server.environment_id,
        server_id: server.server_id,
        revision: server.revision,
        tools: adapter
            .tool_specs()
            .iter()
            .map(|tool| ExtensionTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            })
            .collect(),
    };
    registry
        .register(descriptor)
        .map_err(|_| Error::Registration)?;
    adapter
        .authorize_with_registry(registry, approved_servers)
        .map_err(|_| Error::Authorization)?;
    tools.push(adapter);
    Ok(())
}

impl McpRuntime {
    /// Start and discover only enabled servers. Every discovered server is
    /// registered before it is authorized; a partial failure returns no
    /// runtime so callers cannot accidentally expose a half-built catalog.
    pub async fn build(
        servers: Vec<ConfiguredServer>,
        approved_servers: &HashSet<String>,
        timeout: Duration,
    ) -> Result<Self, Error> {
        Self::build_with_credentials(servers, approved_servers, timeout, Arc::new(NoCredentials))
            .await
    }

    pub async fn build_with_credentials(
        servers: Vec<ConfiguredServer>,
        approved_servers: &HashSet<String>,
        timeout: Duration,
        credentials: Arc<dyn CredentialResolver>,
    ) -> Result<Self, Error> {
        validate_server_identities(&servers)?;
        let mut registry = ExtensionRegistry::default();
        let mut tools = Vec::new();
        let mut identities = Vec::new();
        for server in servers.into_iter().filter(|server| server.enabled) {
            identities.push((
                server.server_id.clone(),
                server.environment_id.clone(),
                server.revision.clone(),
            ));
            let resolved = server
                .credential_handle
                .as_deref()
                .map(|handle| credentials.resolve(handle))
                .transpose()
                .map_err(|_| Error::Authorization)?
                .unwrap_or_default();
            let adapter = match &server.transport {
                Transport::Stdio { .. } => {
                    let mut spec = server.process_spec().ok_or(Error::Process)?;
                    for (key, value) in resolved {
                        if spec.environment.contains_key(&key) {
                            // A configured value must not be able to shadow the
                            // opaque credential channel. Keeping this explicit
                            // also prevents a future resolver from silently
                            // changing the meaning of an existing server config.
                            return Err(Error::Authorization);
                        }
                        spec.environment.insert(key, value);
                    }
                    let client = StdioClient::spawn(spec).map_err(|_| Error::Process)?;
                    McpTools::discover(
                        server.server_id.clone(),
                        server.revision.clone(),
                        client,
                        false,
                        timeout,
                    )
                    .await
                    .map_err(|_| Error::Discovery)?
                }
                Transport::StreamableHttp { url } => {
                    let secret = resolved.get(&OsString::from("RESEARCH_ASSISTANT_MCP_SECRET"));
                    let client =
                        mcp_http::Client::new(url, secret.and_then(|value| value.to_str()))
                            .map_err(|_| Error::Process)?;
                    McpTools::discover_client(
                        server.server_id.clone(),
                        server.revision.clone(),
                        Client::Http(client),
                        false,
                        timeout,
                    )
                    .await
                    .map_err(|_| Error::Discovery)?
                }
            };
            register_adapter(&mut registry, &mut tools, server, adapter, approved_servers)?;
        }
        identities.sort();
        Ok(Self {
            registry,
            tools,
            identities,
        })
    }

    pub fn registry(&self) -> &ExtensionRegistry {
        &self.registry
    }

    pub fn tools(self) -> Vec<McpTools> {
        self.tools
    }

    pub fn definitions(&self) -> Vec<research_agent_core::model::ToolDefinition> {
        self.tools.iter().flat_map(McpTools::definitions).collect()
    }

    /// Identity snapshot of the successfully discovered enabled servers.
    /// Hosts use it to invalidate a runtime when app-owned configuration changes.
    pub fn identities(&self) -> &[(String, String, String)] {
        &self.identities
    }

    fn executor(&self, call: &ToolCall) -> Result<&McpTools, RunError> {
        self.tools
            .iter()
            .find(|tool| tool.handles(&call.name))
            .ok_or(RunError::ToolNotFound)
    }
}

fn validate_server_identities(servers: &[ConfiguredServer]) -> Result<(), Error> {
    let mut ids = HashSet::new();
    for server in servers.iter().filter(|server| server.enabled) {
        if !ids.insert(server.server_id.as_str()) {
            return Err(Error::Registration);
        }
    }
    Ok(())
}

impl McpToolProvider for McpRuntime {
    fn definitions(&self) -> Vec<research_agent_core::model::ToolDefinition> {
        McpRuntime::definitions(self)
    }

    fn approved_servers(&self) -> HashSet<String> {
        self.tools
            .iter()
            .filter(|tool| tool.is_approved())
            .map(|tool| tool.server_id().to_owned())
            .collect()
    }
    fn namespaced_view(self: Arc<Self>) -> Arc<dyn McpToolProvider> {
        let allowed = self.approved_servers();
        Arc::new(self.namespaced(allowed))
    }
}

impl ScopedMcpRuntime {
    fn executor(&self, call: &ToolCall) -> Result<&McpTools, RunError> {
        let tool = self.runtime.executor(call)?;
        if self.allowed_servers.contains(tool.server_id()) {
            Ok(tool)
        } else {
            Err(RunError::ToolRejected)
        }
    }
}

impl McpToolProvider for ScopedMcpRuntime {
    fn definitions(&self) -> Vec<research_agent_core::model::ToolDefinition> {
        self.runtime
            .tools
            .iter()
            .filter(|tool| self.allowed_servers.contains(tool.server_id()))
            .flat_map(McpTools::definitions)
            .collect()
    }

    fn approved_servers(&self) -> HashSet<String> {
        self.allowed_servers.clone()
    }
    fn namespaced_view(self: Arc<Self>) -> Arc<dyn McpToolProvider> {
        Arc::new(self.runtime.namespaced(self.allowed_servers.clone()))
    }
}

impl ToolExecutor for ScopedMcpRuntime {
    fn resolve_name(&self, name: &str) -> ToolNameResolution {
        // A known handler outside this grant remains known; validate rejects
        // that scope. Approval/revocation never changes route identity.
        self.runtime.resolve_name(name)
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        self.executor(call)?.validate(call)
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move { self.executor(&call)?.authorize_and_execute(call).await })
    }

    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        match self.executor(&call) {
            Ok(tool) => tool.result_committed(call),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::items_after_test_module,
    reason = "MCP executor adapter is kept adjacent to its test fixture"
)]
mod tests {
    use super::*;
    use crate::ai::JsonCredentialStore;
    use std::{ffi::OsString, path::PathBuf, sync::Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn builds_registered_and_authorized_runtime_from_host_config() {
        let script = r#"while IFS= read line; do case "$line" in *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;; *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"search","inputSchema":{"type":"object"}}]}}' ;; esac; done"#;
        let servers = vec![ConfiguredServer {
            server_id: "literature".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from(script)],
                directory: std::env::temp_dir(),
            },
            environment: BTreeMap::new(),
            credential_handle: None,
            enabled: true,
        }];
        let approved = HashSet::from(["literature".to_string()]);
        let runtime = McpRuntime::build(servers, &approved, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(runtime.definitions().len(), 1);
        assert!(runtime.registry().get("literature").is_some());
        assert_eq!(
            runtime.identities(),
            &[("literature".into(), "project".into(), "r1".into())]
        );

        let runtime = Arc::new(runtime);
        let empty_scope = runtime.scoped(HashSet::new());
        assert!(empty_scope.definitions().is_empty());
        assert_eq!(
            empty_scope.resolve_name("literature__search"),
            ToolNameResolution::Registered
        );
        assert_eq!(
            empty_scope.resolve_name("literature__missing"),
            ToolNameResolution::NotFound
        );
        assert!(
            empty_scope
                .validate(&ToolCall {
                    namespace: None,
                    raw_arguments_json: None,
                    call_id: "call-scoped".into(),
                    name: "literature__search".into(),
                    arguments: serde_json::json!({}),
                })
                .is_err()
        );
        let approved_scope = runtime.scoped(HashSet::from(["literature".to_owned()]));
        assert_eq!(approved_scope.definitions().len(), 1);
        assert!(
            approved_scope
                .validate(&ToolCall {
                    namespace: None,
                    raw_arguments_json: None,
                    call_id: "call-approved".into(),
                    name: "literature__search".into(),
                    arguments: serde_json::json!({}),
                })
                .is_ok()
        );
    }

    /// Runs against the pinned Codex RMCP test server when its freshly built
    /// binary is supplied by the developer. This is intentionally ignored so
    /// normal CI never depends on a reference checkout or a local toolchain.
    #[tokio::test]
    #[ignore = "requires RA_EXTERNAL_MCP_SERVER pointing at the built external RMCP server"]
    async fn discovers_and_executes_a_real_external_mcp_server() {
        let program = std::env::var_os("RA_EXTERNAL_MCP_SERVER")
            .expect("RA_EXTERNAL_MCP_SERVER must point to rmcp_test_server");
        let server = ConfiguredServer {
            server_id: "external".into(),
            environment_id: "reference-project".into(),
            revision: "pinned-source".into(),
            transport: Transport::Stdio {
                program: PathBuf::from(program),
                arguments: Vec::new(),
                directory: std::env::current_dir().unwrap(),
            },
            environment: BTreeMap::new(),
            credential_handle: None,
            enabled: true,
        };
        let runtime = McpRuntime::build(
            vec![server],
            &HashSet::from(["external".to_owned()]),
            Duration::from_secs(10),
        )
        .await
        .expect("external MCP discovery");
        assert_eq!(runtime.definitions().len(), 1);
        assert_eq!(runtime.definitions()[0].name, "external__echo");
        let call = ToolCall {
            namespace: None,
            raw_arguments_json: None,
            call_id: "external-call".into(),
            name: "external__echo".into(),
            arguments: serde_json::json!({"message":"real external MCP","env_var":"PATH"}),
        };
        let output = runtime
            .authorize_and_execute(call)
            .await
            .expect("external MCP tool call");
        assert!(!output.is_error);
        assert!(output.content.contains("real external MCP"));
    }

    #[tokio::test]
    async fn builds_authorized_runtime_from_streamable_http_config() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for request_index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                loop {
                    let count = stream.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let body = if request_index == 1 {
                    br#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"search","inputSchema":{"type":"object"}}]}}"#.to_vec()
                } else {
                    br#"{"jsonrpc":"2.0","id":1,"result":{}}"#.to_vec()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        let servers = vec![ConfiguredServer {
            server_id: "literature-http".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
            transport: Transport::StreamableHttp {
                url: format!("http://{address}/mcp"),
            },
            environment: Default::default(),
            credential_handle: None,
            enabled: true,
        }];
        let approved = HashSet::from(["literature-http".to_string()]);
        let runtime = McpRuntime::build(servers, &approved, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(runtime.definitions().len(), 1);
        assert!(runtime.registry().get("literature-http").is_some());
        server.await.unwrap();
    }

    #[test]
    fn resolves_mcp_handle_through_existing_private_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credentials.json");
        let store = JsonCredentialStore::from_explicit_development_path(path);
        store
            .set_mcp_for_test("mcp:literature", "secret-value-123")
            .unwrap();
        let resolver = JsonMcpCredentialResolver::new(store);
        let environment = resolver.resolve("mcp:literature").unwrap();
        assert_eq!(
            environment
                .get(&OsString::from("RESEARCH_ASSISTANT_MCP_SECRET"))
                .unwrap(),
            "secret-value-123"
        );
        assert!(resolver.resolve("provider:qwen").is_err());
    }

    #[tokio::test]
    async fn rejects_duplicate_enabled_server_ids_before_spawning_any_process() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("spawned");
        let script = format!("touch '{}' ; while read line; do :; done", marker.display());
        let server = |revision: &str| ConfiguredServer {
            server_id: "duplicate".into(),
            environment_id: "project".into(),
            revision: revision.into(),
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from(script.clone())],
                directory: directory.path().to_owned(),
            },
            environment: BTreeMap::new(),
            credential_handle: None,
            enabled: true,
        };
        let result = McpRuntime::build(
            vec![server("r1"), server("r2")],
            &HashSet::from(["duplicate".to_owned()]),
            Duration::from_secs(1),
        )
        .await;
        assert!(matches!(result, Err(Error::Registration)));
        assert!(!marker.exists(), "duplicate validation must precede spawn");
    }

    #[tokio::test]
    async fn rejects_credential_environment_collisions() {
        let server = ConfiguredServer {
            server_id: "credentialed".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from("exit 0")],
                directory: std::env::temp_dir(),
            },
            environment: BTreeMap::from([(
                OsString::from("RESEARCH_ASSISTANT_MCP_SECRET"),
                OsString::from("configured"),
            )]),
            credential_handle: Some("mcp:fixture".into()),
            enabled: true,
        };
        let result = McpRuntime::build_with_credentials(
            vec![server],
            &HashSet::from(["credentialed".to_owned()]),
            Duration::from_secs(1),
            Arc::new(FixtureCredentials),
        )
        .await;
        assert!(matches!(result, Err(Error::Authorization)));
    }

    struct FixtureCredentials;
    impl CredentialResolver for FixtureCredentials {
        fn resolve(&self, handle: &str) -> Result<BTreeMap<OsString, OsString>, CredentialError> {
            if handle != "mcp:fixture" {
                return Err(CredentialError::Unavailable);
            }
            Ok(BTreeMap::from([(
                OsString::from("RESEARCH_ASSISTANT_MCP_SECRET"),
                OsString::from("fixture-secret"),
            )]))
        }
    }

    struct RotatingCredentials {
        secret: Mutex<&'static str>,
    }

    impl CredentialResolver for RotatingCredentials {
        fn resolve(&self, handle: &str) -> Result<BTreeMap<OsString, OsString>, CredentialError> {
            if handle != "mcp:rotating" {
                return Err(CredentialError::Unavailable);
            }
            Ok(BTreeMap::from([(
                OsString::from("RESEARCH_ASSISTANT_MCP_SECRET"),
                OsString::from(*self.secret.lock().unwrap()),
            )]))
        }
    }

    fn rotating_server() -> ConfiguredServer {
        let script = r#"case "$RESEARCH_ASSISTANT_MCP_SECRET" in old-secret) tool=before_rotation ;; new-secret) tool=after_rotation ;; *) exit 17 ;; esac; while IFS= read line; do case "$line" in *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;; *tools/list*) printf '%s\n' "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"$tool\",\"description\":\"credential-backed tool\",\"inputSchema\":{\"type\":\"object\"}}]}}" ;; esac; done"#;
        ConfiguredServer {
            server_id: "credentialed".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from(script)],
                directory: std::env::temp_dir(),
            },
            environment: BTreeMap::new(),
            credential_handle: Some("mcp:rotating".into()),
            enabled: true,
        }
    }

    #[tokio::test]
    async fn injects_credentials_only_into_stdio_child_before_discovery() {
        let script = r#"test "$RESEARCH_ASSISTANT_MCP_SECRET" = "fixture-secret" || exit 17; while IFS= read line; do case "$line" in *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;; *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"credential-backed search","inputSchema":{"type":"object"}}]}}' ;; esac; done"#;
        let server = ConfiguredServer {
            server_id: "credentialed".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from(script)],
                directory: std::env::temp_dir(),
            },
            environment: BTreeMap::new(),
            credential_handle: Some("mcp:fixture".into()),
            enabled: true,
        };
        let runtime = McpRuntime::build_with_credentials(
            vec![server],
            &HashSet::from(["credentialed".to_owned()]),
            Duration::from_secs(2),
            Arc::new(FixtureCredentials),
        )
        .await
        .expect("credentialed child discovery");
        assert_eq!(runtime.definitions().len(), 1);
        assert!(
            runtime
                .definitions()
                .iter()
                .all(|definition| !definition.description.contains("fixture-secret"))
        );
    }

    #[tokio::test]
    async fn rebuilding_runtime_resolves_rotated_credentials_without_reusing_old_secret() {
        let credentials = Arc::new(RotatingCredentials {
            secret: Mutex::new("old-secret"),
        });
        let approved = HashSet::from(["credentialed".to_owned()]);
        let first = McpRuntime::build_with_credentials(
            vec![rotating_server()],
            &approved,
            Duration::from_secs(2),
            credentials.clone(),
        )
        .await
        .expect("first credentialed discovery");
        assert_eq!(first.definitions()[0].name, "credentialed__before_rotation");
        drop(first);

        *credentials.secret.lock().unwrap() = "new-secret";
        let second = McpRuntime::build_with_credentials(
            vec![rotating_server()],
            &approved,
            Duration::from_secs(2),
            credentials,
        )
        .await
        .expect("rotated credential discovery");
        assert_eq!(second.definitions()[0].name, "credentialed__after_rotation");
        assert!(
            second
                .definitions()
                .iter()
                .all(|definition| !definition.description.contains("secret"))
        );
    }

    #[tokio::test]
    async fn unavailable_mcp_credentials_fail_closed_before_child_spawn() {
        let server = ConfiguredServer {
            server_id: "credentialed".into(),
            environment_id: "project".into(),
            revision: "r1".into(),
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from("exit 0")],
                directory: std::env::temp_dir(),
            },
            environment: BTreeMap::new(),
            credential_handle: Some("mcp:missing".into()),
            enabled: true,
        };
        let result = McpRuntime::build(
            vec![server],
            &HashSet::from(["credentialed".to_owned()]),
            Duration::from_secs(2),
        )
        .await;
        assert!(matches!(result, Err(Error::Authorization)));
    }
}

#[cfg(test)]
#[path = "mcp_runtime/plugin_sharing_tests.rs"]
mod plugin_sharing_tests;

impl ToolExecutor for McpRuntime {
    fn resolve_name(&self, name: &str) -> ToolNameResolution {
        if self.tools.iter().any(|tool| tool.handles(name)) {
            ToolNameResolution::Registered
        } else {
            ToolNameResolution::NotFound
        }
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        self.executor(call)?.validate(call)
    }

    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move { self.executor(&call)?.authorize_and_execute(call).await })
    }

    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        match self.executor(&call) {
            Ok(tool) => tool.result_committed(call),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
}
