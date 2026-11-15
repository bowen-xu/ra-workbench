//! Native task owner -> actual Responses codec -> SQLite window -> next request.
//! All HTTP and accounts below are synthetic; no real credential or provider.
use super::{compaction_tests::*, *};
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
    context::{
        checkpoint::ContextCheckpoint,
        compaction_strategy::{
            CompactionBinding, CompactionCandidate, CompactionFailure, CompactionFuture,
            CompactionKind, CompactionOutput, CompactionPhase, CompactionRequest,
            CompactionRetention, CompactionStrategy,
        },
    },
    conversation::ConversationId,
    runner::{RunError, RunFuture, RunLimits, ToolExecutor, ToolOutput},
    submission::{SubmissionInput, SubmissionRecord, SubmissionStore},
    tool_registry::OwnedToolRegistration,
};
use std::sync::atomic::AtomicBool;
#[path = "native_compaction/accounting.rs"]
mod accounting;
#[path = "native_compaction/authority.rs"]
mod authority;
#[path = "native_compaction/fork.rs"]
mod fork;
#[path = "native_compaction/images.rs"]
mod images;
#[path = "native_compaction/legacy.rs"]
mod legacy;
#[path = "native_compaction/manual_tools.rs"]
mod manual_tools;
#[path = "native_compaction/metadata.rs"]
mod metadata;
#[path = "native_compaction/mid_turn.rs"]
mod mid_turn;
#[path = "native_compaction/pre_turn.rs"]
mod pre_turn;
#[path = "native_compaction/resume.rs"]
mod resume;
#[path = "native_compaction/retention.rs"]
mod retention;
#[path = "native_compaction/retention_failure.rs"]
mod retention_failure;
#[path = "native_compaction/rollback.rs"]
mod rollback;
#[path = "native_compaction/storage_failure.rs"]
mod storage_failure;
#[path = "native_compaction/text_retry.rs"]
mod text_retry;

struct NoTools;
impl ToolExecutor for NoTools {
    fn validate(&self, _: &research_agent_core::model::ToolCall) -> Result<(), RunError> {
        Err(RunError::ToolRejected)
    }
    fn authorize_and_execute(
        &self,
        _: research_agent_core::model::ToolCall,
    ) -> RunFuture<'_, ToolOutput> {
        Box::pin(async { Err(RunError::ToolRejected) })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    app: crate::storage::AppStore,
    service: Arc<AgentSessionService>,
    store: Arc<SqliteSubmissionStore>,
    id: Uuid,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let app = crate::storage::AppStore::for_test(db, root.path().join("objects"));
        let service = Arc::new(AgentSessionService::from_app_store(&app).unwrap());
        let id = Uuid::new_v4();
        service.create(ConversationId::from_uuid(id)).await.unwrap();
        let store = Arc::new(SqliteSubmissionStore::new(app.shared_agent_connection()));
        Self {
            _root: root,
            app,
            service,
            store,
            id,
        }
    }
    async fn record(&self, text: &str) -> SubmissionRecord {
        self.record_with_context(text, vec![]).await
    }
    async fn record_with_context(
        &self,
        text: &str,
        context: Vec<research_agent_core::submission::ContextText>,
    ) -> SubmissionRecord {
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
                    model: request().model,
                    reasoning_effort: None,
                    service_tier: None,
                    context,
                },
            )
            .await
            .unwrap()
    }
    fn transport(&self, record: &SubmissionRecord, url: &str) -> Arc<ResponsesTransport> {
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
        .unwrap();
        Arc::new(transport(url.into(), Arc::new(state)))
    }
    fn execution(
        &self,
        record: &SubmissionRecord,
        transport: Arc<ResponsesTransport>,
    ) -> AgentExecution {
        let strategy = ResponsesV2CompactionStrategy::new(
            transport.clone(),
            CompactionBinding {
                model: record.input.model.clone(),
                strategy_id: "fixture.v2".into(),
                state_schema: 1,
                kind: CompactionKind::RemoteOpaque,
            },
        )
        .unwrap();
        AgentExecution {
            #[cfg(target_os = "macos")]
            stop_hooks: None,
            compaction_hooks: None,
            model: record.input.model.clone(),
            transport,
            compaction_strategy: Some(Arc::new(strategy)),
            host_tools: OwnedToolRegistration::new(vec![], Arc::new(NoTools)),
            system_instructions: "current research authority".into(),
            max_output_tokens: 128,
            max_input_bytes: 100_000,
            limits: RunLimits {
                model_calls: Some(3),
                tool_calls: Some(3),
                output_bytes: Some(10_000),
                tool_output_bytes: 10_000,
                response: research_agent_core::model::StreamLimits {
                    text_bytes: 10_000,
                    argument_bytes: 10_000,
                    tool_calls: 3,
                },
            },
        }
    }
    async fn start(
        &self,
        record: &SubmissionRecord,
        execution: AgentExecution,
        compact: bool,
    ) -> crate::agent::service::OwnedTurn {
        let execution = Arc::new(execution);
        let result = if compact {
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
        let SubmissionAdmission::Started(owned) = result else {
            panic!("new task expected")
        };
        owned
    }
}

#[tokio::test]
async fn native_manual_v2_installs_restores_and_recompacts_without_old_opaque_duplication() {
    let fixture = Fixture::new().await;
    let first_opaque = opaque();
    let second_opaque =
        json!({"type":"compaction","encrypted_content":"replacement state","vendor_field":5});
    let (url, peer) = sequential_server(vec![
        ok(&body()),
        ok(&events(std::slice::from_ref(&first_opaque), true)),
        ok(&body()),
        ok(&events(std::slice::from_ref(&second_opaque), true)),
        ok(&body()),
    ])
    .await;
    let mut windows = vec![];
    for (text, compact) in [
        ("hello", false),
        ("/compact", true),
        ("fresh", false),
        ("/compact", true),
        ("after second", false),
    ] {
        let record = fixture.record(text).await;
        let execution = fixture.execution(&record, fixture.transport(&record, &url));
        assert_eq!(
            fixture
                .start(&record, execution, compact)
                .await
                .task
                .wait()
                .await,
            TaskCompletion::Completed
        );
        if compact {
            let window = fixture
                .service
                .active_context_checkpoint(fixture.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(window.credential_scope_id, Some(Uuid::from_u128(1)));
            assert!(matches!(window.output, CompactionOutput::RemoteOpaque(_)));
            let history = fixture
                .service
                .open(ConversationId::from_uuid(fixture.id))
                .await
                .unwrap();
            let turn = history.turn(record.turn_id).unwrap();
            assert_eq!(turn.items.len(), 1);
            assert!(turn.items[&window.window_id].completed);
            windows.push(window);
        }
    }
    let sent = peer.await.unwrap();
    assert_eq!(sent.len(), 5);
    assert!(
        sent.iter()
            .all(|body| !body.to_string().contains("/compact"))
    );
    assert_eq!(sent[2]["input"][1]["content"][0]["text"], "hello");
    assert_eq!(sent[2]["input"][2], first_opaque);
    assert_eq!(sent[2]["input"][3]["content"][0]["text"], "fresh");
    assert!(sent[3]["input"].as_array().unwrap().contains(&first_opaque));
    assert_eq!(windows[1].previous_window_id, Some(windows[0].window_id));
    assert!(!sent[4]["input"].as_array().unwrap().contains(&first_opaque));
    assert_eq!(sent[4]["input"][3], second_opaque);
    assert_eq!(
        sent[4]["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "compaction")
            .count(),
        1
    );
}

struct HeldValidation {
    inner: Arc<ResponsesTransport>,
    entered: tokio::sync::Notify,
    dropped: Arc<AtomicBool>,
}
struct ValidationGuard(Arc<AtomicBool>);
impl Drop for ValidationGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl ModelTransport for HeldValidation {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn supports_compaction_kind(&self, kind: CompactionKind) -> bool {
        self.inner.supports_compaction_kind(kind)
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        self.inner.start(request)
    }
    fn validate_compaction_window<'a>(
        &'a self,
        _: &'a ContextCheckpoint,
        _: &'a ModelRequest,
    ) -> ModelFuture<'a, ()> {
        Box::pin(async move {
            let _guard = ValidationGuard(self.dropped.clone());
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}

#[tokio::test]
async fn native_remote_cancel_after_candidate_before_validation_cannot_install() {
    let fixture = Fixture::new().await;
    let (url, peer) = server(ok(&events(&[opaque()], true)), false).await;
    let record = fixture.record("/compact").await;
    let inner = fixture.transport(&record, &url);
    let held = Arc::new(HeldValidation {
        inner: inner.clone(),
        entered: tokio::sync::Notify::new(),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let mut execution = fixture.execution(&record, inner);
    execution.transport = held.clone();
    let owned = fixture.start(&record, execution, true).await;
    timeout(Duration::from_secs(3), held.entered.notified())
        .await
        .unwrap();
    fixture
        .service
        .tasks()
        .cancel_turn(ConversationId::from_uuid(fixture.id), owned.turn_id)
        .unwrap();
    assert_eq!(
        timeout(Duration::from_secs(3), owned.task.wait())
            .await
            .unwrap(),
        TaskCompletion::Canceled
    );
    assert!(held.dropped.load(Ordering::SeqCst));
    assert!(
        fixture
            .service
            .active_context_checkpoint(fixture.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        peer.await.unwrap()["input"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["type"],
        "compaction_trigger"
    );
}

#[tokio::test]
async fn native_remote_candidate_from_different_sampling_account_is_not_installed() {
    let fixture = Fixture::new().await;
    let (url, peer) = server(ok(&events(&[opaque()], true)), false).await;
    let record = fixture.record("/compact").await;
    let mut execution = fixture.execution(&record, fixture.transport(&record, &url));
    let mut other = fixture.transport(&record, &url);
    Arc::get_mut(&mut other).unwrap().config.credential_scope_id = Uuid::from_u128(2);
    execution.transport = other;
    let completion = fixture
        .start(&record, execution, true)
        .await
        .task
        .wait()
        .await;
    assert_eq!(completion, TaskCompletion::Failed);
    assert!(
        fixture
            .service
            .active_context_checkpoint(fixture.id)
            .await
            .unwrap()
            .is_none()
    );
    peer.await.unwrap();
}

struct HeldCandidate {
    inner: Arc<dyn CompactionStrategy>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl CompactionStrategy for HeldCandidate {
    fn binding(&self) -> CompactionBinding {
        self.inner.binding()
    }
    fn retention_policy(&self) -> Option<CompactionRetention> {
        self.inner.retention_policy()
    }
    fn retained_text_policy(
        &self,
    ) -> Option<Arc<dyn research_agent_core::context::retained_text::RetainedTextPolicy>> {
        self.inner.retained_text_policy()
    }
    fn validate_output(&self, output: &CompactionOutput) -> Result<(), CompactionFailure> {
        self.inner.validate_output(output)
    }
    fn compact(&self, request: CompactionRequest) -> CompactionFuture<'_, CompactionCandidate> {
        Box::pin(async move {
            let result = self.inner.compact(request).await?;
            self.entered.notify_one();
            self.release.notified().await;
            Ok(result)
        })
    }
}
