use super::*;
use crate::agent::{
    compaction_hook_commands::{ApproveRequest, Host, RevokeRequest, Scope},
    composed_tools::RejectingToolExecutor,
    host_execution,
    service::AgentSessionService,
};
use research_agent_core::{
    context::compaction_strategy::CompactionPhase, model::ToolCall,
    tool_registry::OwnedToolRegistration,
};
#[path = "tests/stop.rs"]
mod stop;

struct Fixture {
    _root: tempfile::TempDir,
    store: crate::storage::AppStore,
    record: SubmissionRecord,
    settings: Arc<HookConsentService>,
    factory: NativeHookFactory,
    sessions: Arc<AgentSessionService>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_data_interface(false)
    }
    fn managed() -> Self {
        Self::with_data_interface(true)
    }
    fn with_data_interface(managed: bool) -> Self {
        let (root, store, record) = if managed {
            host_execution::tests::managed_fixture("note")
        } else {
            host_execution::tests::fixture("note")
        };
        crate::storage::migrate(&mut store.connection().unwrap()).unwrap();
        host_execution::freeze_grant_sql(&store.connection().unwrap(), &record).unwrap();
        let settings = Arc::new(HookConsentService::from_app_store(&store));
        let factory = NativeHookFactory::new(settings.clone());
        let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
        sessions
            .processes()
            .begin_turn(record.conversation_id, record.turn_id)
            .unwrap();
        Self {
            _root: root,
            store,
            record,
            settings,
            factory,
            sessions,
        }
    }
    fn scope(&self) -> Scope {
        Scope {
            host: Host::Notes,
            model: self.record.input.model.clone(),
        }
    }
    fn write(&self, enabled: bool, stage: &str, command: &str) {
        let root = self.store.agent_configuration_directory().join("agent");
        std::fs::create_dir_all(&root).unwrap();
        let command = toml::Value::String(command.into());
        std::fs::write(root.join("hooks.toml"), format!(
            "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='{stage}'\nenabled={enabled}\nprogram='/bin/sh'\narguments=['-c', {command}]\ntimeout_ms=5000\nmax_output_bytes=1024\n"
        )).unwrap();
    }
    fn approve(&self) {
        for review in self.settings.review(self.scope()).unwrap().handlers {
            self.settings
                .approve(ApproveRequest {
                    scope: self.scope(),
                    handler_id: review.handler_id,
                    expected_revision: review.revision,
                    fingerprint: review.fingerprint,
                })
                .unwrap();
        }
    }
    async fn tools(&self) -> host_execution::HostTools {
        host_execution::registration_with_hooks(
            self.store.clone(),
            self.record.clone(),
            self.sessions.clone(),
            OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
        )
        .await
        .unwrap()
    }
    fn request(&self, stage: CompactionHookStage) -> CompactionHookRequest {
        CompactionHookRequest {
            operation_id: Uuid::new_v4(),
            conversation_id: self.record.conversation_id,
            turn_id: self.record.turn_id,
            phase: CompactionPhase::Manual,
            model: self.record.input.model.clone(),
            stage,
        }
    }
    fn work(&self) -> PathBuf {
        self.store
            .agent_workspace_directory()
            .join(self.record.turn_id.to_string())
            .join("work")
    }
}

#[tokio::test]
async fn compact_matchers_select_before_request_authorization_and_dispatch() {
    let f = Fixture::new();
    let root = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&root).unwrap();
    let entry = |id: u8, stage: &str, matcher: &str| {
        format!(
            "[[handlers]]\nid='00000000-0000-0000-0000-{id:012}'\nstage='{stage}'\nenabled=true\nmatcher='{matcher}'\nprogram='/bin/sh'\narguments=['-c','cat >/dev/null; touch matched']\ntimeout_ms=5000\nmax_output_bytes=1024\n"
        )
    };
    std::fs::write(
        root.join("hooks.toml"),
        format!(
            "schema_version=1\n{}{}{}{}",
            entry(1, "pre_compact", "manual"),
            entry(2, "pre_compact", "auto"),
            entry(3, "session_start_compact", "compact"),
            entry(4, "post_compact", "[")
        ),
    )
    .unwrap();
    let reviews = f.settings.review(f.scope()).unwrap().handlers;
    assert_eq!(reviews.len(), 3);
    for review in reviews
        .iter()
        .filter(|h| h.handler_id != Uuid::from_u128(2))
    {
        f.settings
            .approve(ApproveRequest {
                scope: f.scope(),
                handler_id: review.handler_id,
                expected_revision: review.revision,
                fingerprint: review.fingerprint.clone(),
            })
            .unwrap();
    }
    let tools = f.tools().await;
    let hooks = f
        .factory
        .prepare(
            AgentHost::Notes,
            &f.record,
            tools.workspace,
            Some(HookContextBudget::new(f.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    let mut request = f.request(CompactionHookStage::PreCompact);
    assert_eq!(hooks.handler_ids_for(&request), [Uuid::from_u128(1)]);
    assert!(hooks.validate_request(&request).is_ok());
    assert_eq!(
        hooks.run_with_context(request.clone()).await.outcome,
        CompactionHookOutcome::Continue
    );
    assert!(f.work().join("matched").exists());
    request.phase = CompactionPhase::MidTurn;
    assert_eq!(hooks.handler_ids_for(&request), [Uuid::from_u128(2)]);
    assert!(hooks.validate_request(&request).is_err());
    request.stage = CompactionHookStage::SessionStartCompact;
    assert_eq!(hooks.handler_ids_for(&request), [Uuid::from_u128(3)]);
    assert!(hooks.validate_request(&request).is_ok());
}

#[tokio::test]
async fn preparation_never_initializes_a_workspace_and_requires_exact_consent() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .factory
            .prepare(
                AgentHost::Notes,
                &fixture.record,
                None,
                Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap())
            )
            .await
            .unwrap()
            .is_none()
    );
    fixture.write(false, "pre_compact", "cat >/dev/null; touch forbidden");
    assert!(
        fixture
            .factory
            .prepare(
                AgentHost::Notes,
                &fixture.record,
                None,
                Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap())
            )
            .await
            .unwrap()
            .is_none()
    );
    fixture.write(true, "pre_compact", "cat >/dev/null; touch approved");
    let tools = fixture.tools().await;
    assert!(
        fixture
            .factory
            .prepare(
                AgentHost::Notes,
                &fixture.record,
                tools.workspace.clone(),
                Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap())
            )
            .await
            .is_err()
    );
    fixture.approve();
    assert!(
        fixture
            .factory
            .prepare(
                AgentHost::Notes,
                &fixture.record,
                None,
                Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap())
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .factory
            .prepare(
                AgentHost::Reading,
                &fixture.record,
                tools.workspace.clone(),
                Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap())
            )
            .await
            .is_err()
    );
    let mut other = fixture.record.clone();
    other.input.model.revision = Some("other".into());
    assert!(
        fixture
            .factory
            .prepare(
                AgentHost::Notes,
                &other,
                tools.workspace.clone(),
                Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap())
            )
            .await
            .is_err()
    );
    let hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            tools.workspace,
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        hooks.handler_ids(CompactionHookStage::PreCompact),
        vec![Uuid::from_u128(0x12)]
    );
    assert!(!fixture.work().exists());
    assert_eq!(
        hooks
            .run(fixture.request(CompactionHookStage::PostCompact))
            .await,
        CompactionHookOutcome::Continue
    );
    assert!(!fixture.work().exists());
}

#[tokio::test]
async fn real_hook_uses_frozen_command_and_tools_original_lazy_lease() {
    let fixture = Fixture::new();
    fixture.write(
        true,
        "pre_compact",
        "cat > hook-input.json; printf original > from-hook.txt; printf '{\"continue\":false}'",
    );
    fixture.approve();
    let tools = fixture.tools().await;
    let hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            tools.workspace,
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(!fixture.work().exists());
    fixture.write(true, "pre_compact", "cat >/dev/null; touch changed-command");
    assert_eq!(
        hooks
            .run(fixture.request(CompactionHookStage::PreCompact))
            .await,
        CompactionHookOutcome::Stop
    );
    assert_eq!(
        std::fs::read_to_string(fixture.work().join("from-hook.txt")).unwrap(),
        "original"
    );
    assert!(!fixture.work().join("changed-command").exists());
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.work().join("hook-input.json")).unwrap())
            .unwrap();
    assert_eq!(input["cwd"], fixture.work().to_str().unwrap());
    let (_, executor) = tools.tools.into_parts();
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "read-hook-result".into(),
        name: "research_exec_command".into(),
        arguments: serde_json::json!({"command":"cat from-hook.txt","yield_ms":5000}),
    };
    let result = executor.authorize_and_execute(call.clone()).await.unwrap();
    assert!(
        !result.is_error && result.content.contains("original"),
        "{}",
        result.content
    );
    executor.result_committed(call).await.unwrap();
    fixture.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn revoked_hook_or_host_grants_cannot_initialize_or_execute_old_plans() {
    let fixture = Fixture::new();
    fixture.write(true, "pre_compact", "cat >/dev/null; touch forbidden");
    fixture.approve();
    let tools = fixture.tools().await;
    let hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            tools.workspace,
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    fixture
        .settings
        .revoke(RevokeRequest {
            scope: fixture.scope(),
            handler_id: Uuid::from_u128(0x12),
            expected_revision: 1,
        })
        .unwrap();
    assert!(
        hooks
            .validate_request(&fixture.request(CompactionHookStage::PreCompact))
            .is_ok()
    );
    hooks
        .run(fixture.request(CompactionHookStage::PreCompact))
        .await;
    assert!(!fixture.work().exists());
    fixture.approve();
    fixture
        .store
        .connection()
        .unwrap()
        .execute("delete from agent_host_shell_grants", [])
        .unwrap();
    hooks
        .run(fixture.request(CompactionHookStage::PreCompact))
        .await;
    assert!(!fixture.work().exists());
}

#[tokio::test]
async fn session_start_context_uses_same_sandbox_and_revocation_is_live() {
    let fixture = Fixture::new();
    fixture.write(
        true,
        "session_start_compact",
        "cat >/dev/null; printf restored-context",
    );
    fixture.approve();
    let tools = fixture.tools().await;
    let hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            tools.workspace,
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    let result = hooks
        .run_with_context(fixture.request(CompactionHookStage::SessionStartCompact))
        .await;
    assert_eq!(result.additional_contexts, vec!["restored-context"]);
    fixture
        .store
        .connection()
        .unwrap()
        .execute("delete from agent_host_shell_grants", [])
        .unwrap();
    let result = hooks
        .run_with_context(fixture.request(CompactionHookStage::SessionStartCompact))
        .await;
    assert!(result.additional_contexts.is_empty());
    fixture.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn multiple_handlers_preserve_context_order_without_competing_writer_intents() {
    let fixture = Fixture::new();
    fixture.write(
        true,
        "session_start_compact",
        "cat >/dev/null; printf first",
    );
    let path = fixture
        .store
        .agent_configuration_directory()
        .join("agent/hooks.toml");
    let first = std::fs::read_to_string(&path).unwrap();
    let second = first
        .split_once("[[handlers]]")
        .unwrap()
        .1
        .replace("000000000012", "000000000013")
        .replace("printf first", "printf second");
    std::fs::write(&path, format!("{first}\n[[handlers]]{second}")).unwrap();
    fixture.approve();
    let tools = fixture.tools().await;
    let hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            tools.workspace,
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    let result = hooks
        .run_with_context(fixture.request(CompactionHookStage::SessionStartCompact))
        .await;
    assert_eq!(result.additional_contexts, vec!["first", "second"]);
    assert!(
        !fixture
            .work()
            .parent()
            .unwrap()
            .join("execution.json")
            .exists()
    );
    fixture.sessions.processes().shutdown().await.unwrap();
}

struct PausedWorkspace {
    inner: Arc<dyn HookWorkspaceSource>,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl HookWorkspaceSource for PausedWorkspace {
    fn prepare<'a>(&'a self, record: &'a SubmissionRecord) -> ModelFuture<'a, HookWorkspace> {
        Box::pin(async move {
            self.entered.notify_one();
            self.resume.notified().await;
            self.inner.prepare(record).await
        })
    }
}

#[tokio::test]
async fn revocation_during_lazy_preparation_is_checked_again_before_launch() {
    let fixture = Fixture::new();
    fixture.write(true, "pre_compact", "cat >/dev/null; touch forbidden");
    fixture.approve();
    let tools = fixture.tools().await;
    let workspace = Arc::new(PausedWorkspace {
        inner: tools.workspace.unwrap(),
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    let hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            Some(workspace.clone()),
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    let mut pending = hooks.run(fixture.request(CompactionHookStage::PreCompact));
    tokio::select! { _ = workspace.entered.notified() => {}, _ = &mut pending => panic!("hook must wait for its original workspace") }
    fixture
        .settings
        .revoke(RevokeRequest {
            scope: fixture.scope(),
            handler_id: Uuid::from_u128(0x12),
            expected_revision: 1,
        })
        .unwrap();
    workspace.resume.notify_one();
    pending.await;
    assert!(!fixture.work().join("forbidden").exists());
    assert!(
        !fixture
            .work()
            .parent()
            .unwrap()
            .join("execution.json")
            .exists()
    );
    fixture.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn cancellation_while_waiting_for_workspace_cannot_start_it_later() {
    let fixture = Fixture::new();
    fixture.write(true, "pre_compact", "cat >/dev/null; touch forbidden");
    fixture.approve();
    let tools = fixture.tools().await;
    let workspace = Arc::new(PausedWorkspace {
        inner: tools.workspace.unwrap(),
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    let hooks = fixture
        .factory
        .prepare(
            AgentHost::Notes,
            &fixture.record,
            Some(workspace.clone()),
            Some(HookContextBudget::new(fixture.record.input.model.clone(), 512).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    let mut pending = hooks.run(fixture.request(CompactionHookStage::PreCompact));
    tokio::select! { _ = workspace.entered.notified() => {}, _ = &mut pending => panic!("hook must wait for its original workspace") }
    drop(pending);
    workspace.resume.notify_one();
    assert!(!fixture.work().exists());
}

#[path = "tests/spill.rs"]
mod spill;
