use super::*;
use crate::agent::execution_profile::{
    accounting_transport::AccountingTransport, compaction::NativeReadStrategy,
};
use crate::agent::model::responses_transport::{ResponsesTransport, ResponsesTransportConfig};
use crate::agent::{
    execution::AgentExecution,
    repository::{
        SqliteSubmissionStore,
        responses_state::{ResponsesStateScope, SqliteResponsesState},
    },
    service::{AgentSessionService, SubmissionAdmission},
    tasks::TaskCompletion,
};
use research_agent_core::{
    context::accounting::{ContextAccounting, Utf8JsonTokenEstimator},
    conversation::ConversationId,
    runner::{RunError, RunFuture, RunLimits, ToolExecutor, ToolOutput},
    submission::*,
    tool_registry::OwnedToolRegistration,
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use uuid::Uuid;

pub(super) fn model(revision: &str) -> ModelIdentity {
    ModelIdentity {
        provider_id: "openai".into(),
        model_id: if revision == "current" {
            "gpt-5.6-sol"
        } else {
            "gpt-5.6-terra"
        }
        .into(),
        revision: Some(revision.into()),
    }
}
pub(super) fn wire(mime: &str, body: String) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn events_for(id: &str, output: &[Value], model: &str) -> String {
    let mut values = vec![json!({"type":"response.created","response":{"id":id,"model":model}})];
    for (index, item) in output.iter().enumerate() {
        values.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
    }
    values.push(json!({"type":"response.completed","response":{"id":id,"model":model,"status":"completed","output":output}}));
    values
        .into_iter()
        .map(|e| format!("data: {e}\n\n"))
        .collect()
}
pub(super) fn sample(id: &str) -> String {
    sample_for(id, "gpt-5.6-terra")
}
pub(super) fn sample_for(id: &str, model: &str) -> String {
    wire(
        "text/event-stream",
        events_for(
            id,
            &[
                json!({"type":"message","id":id,"role":"assistant","content":[{"type":"output_text","text":"answer"}]}),
            ],
            model,
        ),
    )
}
pub(super) fn compact(native: NativeCompactionPolicy, item: &Value) -> String {
    compact_for(native, item, "gpt-5.6-terra")
}
pub(super) fn compact_for(native: NativeCompactionPolicy, item: &Value, model: &str) -> String {
    if native == NativeCompactionPolicy::ChatGptResponsesV2 {
        wire(
            "text/event-stream",
            events_for("compact-response", std::slice::from_ref(item), model),
        )
    } else {
        wire(
            "application/json",
            json!({"model":model,"output":[item]}).to_string(),
        )
    }
}
pub(super) async fn server(
    responses: Vec<String>,
) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), async move {
            let mut requests = vec![];
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![];
                let (start, size) = loop {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let size: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + size {
                            break (end + 4, size);
                        }
                    }
                };
                let request: Value = serde_json::from_slice(&bytes[start..start + size]).unwrap();
                requests.push(request);
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        })
        .await
        .unwrap()
    });
    (url, task)
}
struct NoTools;
impl ToolExecutor for NoTools {
    fn validate(&self, _: &ToolCall) -> Result<(), RunError> {
        Err(RunError::ToolRejected)
    }
    fn authorize_and_execute(&self, _: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async { Err(RunError::ToolRejected) })
    }
}
pub(super) struct Fixture {
    _root: tempfile::TempDir,
    path: std::path::PathBuf,
    app: AppStore,
    pub service: Arc<AgentSessionService>,
    store: Arc<SqliteSubmissionStore>,
    pub id: Uuid,
}
impl Fixture {
    pub async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("task.sqlite");
        let mut db = rusqlite::Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let app = AppStore::for_test(db, root.path().join("objects"));
        let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
        let id = Uuid::new_v4();
        service.create(ConversationId::from_uuid(id)).await.unwrap();
        let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
        Self {
            _root: root,
            path,
            app,
            service,
            store,
            id,
        }
    }
    pub async fn reopen(&mut self) {
        self.service.shutdown().await.unwrap();
        self.app.shared_agent_connection().shutdown().unwrap();
        let mut connection = rusqlite::Connection::open(&self.path).unwrap();
        crate::storage::migrate(&mut connection).unwrap();
        self.app = AppStore::for_test(connection, self._root.path().join("objects"));
        self.service = Arc::new(AgentSessionService::from_app_store(&self.app).unwrap());
        self.store = Arc::new(SqliteSubmissionStore::new(
            self.app.shared_agent_connection(),
        ));
    }
    pub async fn record(&self, model: ModelIdentity, text: &str) -> SubmissionRecord {
        self.store
            .put(
                self.id,
                Uuid::new_v4(),
                SubmissionInput {
                    remote_compaction_v2: None,
                    user_input: vec![],
                    skill_instructions: vec![],
                    instructions: vec![],
                    instruction_configs: vec![],
                    skills: vec![],
                    text: text.into(),
                    model,
                    reasoning_effort: None,
                    service_tier: None,
                    context: vec![],
                },
            )
            .await
            .unwrap()
    }
    pub fn transport(
        &self,
        record: &SubmissionRecord,
        url: &str,
        native: NativeCompactionPolicy,
    ) -> Arc<ResponsesTransport> {
        self.transport_with_account(record, url, native, Uuid::from_u128(1))
    }
    pub fn transport_with_account(
        &self,
        record: &SubmissionRecord,
        url: &str,
        native: NativeCompactionPolicy,
        account: Uuid,
    ) -> Arc<ResponsesTransport> {
        let state = SqliteResponsesState::new(
            self.app.shared_agent_connection(),
            ResponsesStateScope {
                conversation_id: self.id,
                turn_id: record.turn_id,
                context_snapshot_id: record.context_snapshot_id,
                model: record.input.model.clone(),
            },
            100_000,
        )
        .unwrap()
        .with_native_read_policy(native);
        Arc::new(
            ResponsesTransport::synthetic_platform(
                ResponsesTransportConfig {
                    credential_scope_id: account,
                    provider_id: "openai".into(),
                    service_tier: None,
                    timeout: Duration::from_secs(3),
                    capabilities: ModelCapabilities {
                        vision: false,
                        tools: true,
                        parallel_tools: false,
                        reasoning_efforts: vec![],
                    },
                },
                url,
                Arc::new(state),
            )
            .unwrap(),
        )
    }
    pub fn execution(
        &self,
        record: &SubmissionRecord,
        raw: Arc<ResponsesTransport>,
        native: NativeCompactionPolicy,
    ) -> AgentExecution {
        self.execution_with_accounting(
            record,
            raw,
            native,
            Arc::new(ContextAccounting {
                model: record.input.model.clone(),
                budget: None,
                unknown_window_trigger_tokens: None,
                estimator: Arc::new(Utf8JsonTokenEstimator),
            }),
        )
    }
    pub fn execution_with_accounting(
        &self,
        record: &SubmissionRecord,
        raw: Arc<ResponsesTransport>,
        native: NativeCompactionPolicy,
        accounting: Arc<ContextAccounting>,
    ) -> AgentExecution {
        let strategy: Arc<dyn CompactionStrategy> = Arc::new(NativeReadStrategy {
            inner: native
                .responses_strategy_with_accounting(
                    raw.clone(),
                    record.input.model.clone(),
                    accounting.clone(),
                )
                .unwrap(),
            policy: native,
        });
        let mut frozen = policy("unused", None);
        frozen.binding = strategy.binding();
        frozen.authentication = ContextAuthentication::CodexBackend;
        frozen.max_output_tokens = 128;
        frozen.unknown_window_trigger_tokens = None;
        frozen.budget = accounting.budget;
        let saved =
            SqliteContextPolicy::new(self.app.shared_agent_connection(), record, frozen).unwrap();
        let transport = Arc::new(AccountingTransport::new(
            saved,
            None,
            raw,
            accounting,
            strategy.clone(),
            None,
        ));
        AgentExecution {
            #[cfg(target_os = "macos")]
            stop_hooks: None,
            compaction_hooks: None,
            model: record.input.model.clone(),
            transport,
            compaction_strategy: Some(strategy),
            host_tools: OwnedToolRegistration::new(vec![], Arc::new(NoTools)),
            system_instructions: "current authority".into(),
            max_output_tokens: 128,
            max_input_bytes: 100_000,
            limits: RunLimits {
                model_calls: Some(3),
                tool_calls: Some(3),
                output_bytes: Some(10_000),
                tool_output_bytes: 10_000,
                response: StreamLimits {
                    text_bytes: 10_000,
                    argument_bytes: 10_000,
                    tool_calls: 3,
                },
            },
        }
    }
    pub async fn run(&self, record: &SubmissionRecord, execution: AgentExecution, compact: bool) {
        self.run_expecting(record, execution, compact, TaskCompletion::Completed)
            .await;
    }
    pub async fn run_expecting(
        &self,
        record: &SubmissionRecord,
        execution: AgentExecution,
        compact: bool,
        expected: TaskCompletion,
    ) {
        let owned = self.start(record, execution, compact).await;
        let completion = tokio::time::timeout(Duration::from_secs(5), owned.task.wait())
            .await
            .unwrap();
        assert_eq!(
            completion,
            expected,
            "input={} turn={:?}",
            record.input.text,
            self.service
                .open(ConversationId::from_uuid(self.id))
                .await
                .unwrap()
                .turn(record.turn_id)
        );
    }
    pub async fn start(
        &self,
        record: &SubmissionRecord,
        execution: AgentExecution,
        compact: bool,
    ) -> crate::agent::service::OwnedTurn {
        let execution = Arc::new(execution);
        let admission = if compact {
            execution
                .start_compaction(
                    self.service.clone(),
                    self.store.clone(),
                    ConversationId::from_uuid(self.id),
                    record.request_id,
                    None,
                )
                .await
        } else {
            execution
                .start(
                    self.service.clone(),
                    self.store.clone(),
                    ConversationId::from_uuid(self.id),
                    record.request_id,
                )
                .await
        }
        .unwrap();
        let SubmissionAdmission::Started(owned) = admission else {
            panic!("task did not start")
        };
        owned
    }
    pub fn events(&self, turn: Uuid) -> Vec<research_agent_core::protocol::AgentEvent> {
        let db = self.app.shared_agent_connection();
        let connection = db.lock().unwrap();
        let mut statement = connection.prepare("select payload_json from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 order by ordinal").unwrap();
        statement
            .query_map(
                rusqlite::params![self.id.to_string(), turn.to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
            .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
            .collect()
    }
}

/// Only the old-model selection callback is mocked. All other ports delegate
/// to native AccountingTransport and its original live credential/wire checks.
pub(super) struct Selected {
    pub inner: Arc<dyn ModelTransport>,
    pub selected: Arc<dyn CompactionStrategy>,
}
impl ModelTransport for Selected {
    fn pre_turn_compaction<'a>(
        &'a self,
        _: Uuid,
        _: &'a ModelIdentity,
        _: Option<u64>,
    ) -> ModelFuture<'a, Option<Arc<dyn CompactionStrategy>>> {
        Box::pin(async { Ok(Some(self.selected.clone())) })
    }
    fn prepare_context_policy(&self) -> ModelFuture<'_, ()> {
        self.inner.prepare_context_policy()
    }
    fn authorize_request<'a>(&'a self, request: &'a ModelRequest) -> ModelFuture<'a, ()> {
        self.inner.authorize_request(request)
    }
    fn prepared_compaction_strategy(&self) -> Option<Arc<dyn CompactionStrategy>> {
        self.inner.prepared_compaction_strategy()
    }
    fn supports_compaction_kind(&self, kind: CompactionKind) -> bool {
        self.inner.supports_compaction_kind(kind)
    }
    fn validate_compaction_window<'a>(
        &'a self,
        checkpoint: &'a ContextCheckpoint,
        request: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        self.inner.validate_compaction_window(checkpoint, request)
    }
    fn context_accounting(&self) -> Option<Arc<ContextAccounting>> {
        self.inner.context_accounting()
    }
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
}
