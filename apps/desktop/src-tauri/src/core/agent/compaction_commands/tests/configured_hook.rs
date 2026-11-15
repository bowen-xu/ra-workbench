use super::*;
use crate::agent::{
    compaction_hook_config::consent::SqliteHookConsent, repository::compaction_hook_trust::GrantKey,
};
use crate::agent::{
    compaction_hook_config::{
        self, ConfiguredHook, HookConfigError, HookInvocationAuthority, factory::*,
    },
    compaction_hook_process::AuthorizedHookCommand,
    instructions::InstructionRootIdentity,
};
use research_agent_core::context::compaction_hooks::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "configured_hook/failed_command.rs"]
mod failed_command;

struct Authority {
    root: PathBuf,
    record: SubmissionRecord,
    revision: String,
    stage: CompactionHookStage,
    calls: AtomicUsize,
}
impl ConfigurationAuthority for Authority {
    fn authorize_source(
        &self,
        root: &InstructionRootIdentity<'_>,
        host: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<(), HookConfigError> {
        assert_eq!(root.canonical_path, self.root);
        assert!(matches!(host, AgentHost::Notes));
        assert_eq!(record, &self.record);
        Ok(())
    }
    fn authorize_command(
        &self,
        root: &InstructionRootIdentity<'_>,
        handler: &ConfiguredHook,
        revision: &str,
        _: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<AuthorizedHookCommand, HookConfigError> {
        assert_eq!(root.canonical_path, self.root);
        assert_eq!(record, &self.record);
        assert_eq!(revision, self.revision);
        assert_eq!(
            handler.stage,
            crate::agent::compaction_hook_config::ConfiguredHookStage::Compaction(self.stage)
        );
        assert!(!self.root.join("input.json").exists());
        Ok(AuthorizedHookCommand {
            process: handler.candidate()?,
            transcript_path: None,
            launcher: None,
        })
    }
}
impl HookInvocationAuthority for Authority {
    fn check(
        &self,
        _: Uuid,
        revision: &str,
        request: &CompactionHookRequest,
    ) -> Result<(), HookConfigError> {
        assert_eq!(revision, self.revision);
        assert_eq!(request.conversation_id, self.record.conversation_id);
        assert_eq!(request.turn_id, self.record.turn_id);
        assert_eq!(request.model, self.record.input.model);
        assert_eq!(request.stage, self.stage);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct Observer;
impl HookHandlerObserver for Observer {
    fn notify(&self, _: Uuid, _: &CompactionHookRequest, _: CompactionHookNotification) {}
}

#[tokio::test]
async fn configured_precompact_command_stops_manual_entry_before_model_and_checkpoint() {
    stopped_command(CompactionHookStage::PreCompact, false).await;
}

#[tokio::test]
async fn configured_postcompact_command_stops_manual_entry_after_checkpoint_installation() {
    stopped_command(CompactionHookStage::PostCompact, false).await;
}

#[tokio::test]
async fn configured_session_start_runs_in_followup_and_preserves_stopped_context() {
    stopped_command(CompactionHookStage::SessionStartCompact, false).await;
}

#[tokio::test]
async fn consented_precompact_preserves_manual_stop_before_installation() {
    stopped_command(CompactionHookStage::PreCompact, true).await;
}
#[tokio::test]
async fn consented_postcompact_preserves_manual_stop_after_installation() {
    stopped_command(CompactionHookStage::PostCompact, true).await;
}
#[tokio::test]
async fn consented_session_start_preserves_followup_and_stopped_context() {
    stopped_command(CompactionHookStage::SessionStartCompact, true).await;
}

fn configured_factory(
    root: PathBuf,
    authority: Arc<Authority>,
    consent: Option<Arc<SqliteHookConsent>>,
) -> Arc<dyn crate::agent::compaction_hook_factory::CompactionHookFactory> {
    let factory = ConfiguredHookFactory::new(root, authority, Arc::new(Observer)).unwrap();
    match consent {
        Some(consent) => Arc::new(factory.with_consent(consent)),
        None => Arc::new(factory),
    }
}

async fn stopped_command(stage: CompactionHookStage, durable: bool) {
    let f = fixture("/compact").await;
    let root = f._root.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("agent")).unwrap();
    let (configured_stage, event_name) = match stage {
        CompactionHookStage::PreCompact => ("pre_compact", "PreCompact"),
        CompactionHookStage::PostCompact => ("post_compact", "PostCompact"),
        CompactionHookStage::SessionStartCompact => ("session_start_compact", "SessionStart"),
    };
    let output = if stage == CompactionHookStage::SessionStartCompact {
        r#"{"continue":false,"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"restored configured context"}}"#
    } else {
        r#"{"continue":false}"#
    };
    let config = format!(
        "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='{configured_stage}'\nenabled=true\nprogram='/bin/sh'\ndirectory='{}'\narguments=['-c', '''cat > input.json; printf '%s' '{output}' ''']\ntimeout_ms=2000\nmax_output_bytes=1024\n",
        root.display()
    );
    std::fs::write(root.join("agent/hooks.toml"), &config).unwrap();
    let authority = Arc::new(Authority {
        root: root.clone(),
        record: f.record.clone(),
        stage,
        revision: compaction_hook_config::parse(&config)
            .unwrap()
            .revision()
            .into(),
        calls: AtomicUsize::new(0),
    });
    let consent = durable.then(|| {
        let consent = Arc::new(SqliteHookConsent::new(f.store.shared_agent_connection()));
        let key = GrantKey::new(
            &root,
            Uuid::from_u128(0x12),
            AgentHost::Notes,
            f.record.input.model.clone(),
        )
        .unwrap();
        crate::agent::instructions::inspect_instruction_root(&root, |identity| {
            consent.update(&key, 0, identity, &authority.revision, true)
        })
        .unwrap()
        .unwrap();
        consent
    });
    let factory = configured_factory(root.clone(), authority.clone(), consent.clone());
    let models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(root.join("unused-credentials.json")),
    )
    .with_compaction_hooks(factory);
    let transport = transport(&f, false);
    let admission = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |record, _| {
            let models = &models;
            let transport = transport.clone();
            async move {
                let hooks = models
                    .prepare_compaction_hooks(AgentHost::Notes, &record)
                    .await
                    .map_err(|_| Error::ModelUnavailable)?;
                assert!(!root.join("input.json").exists());
                Ok((transport as Arc<dyn ModelTransport>, hooks, empty_tools()))
            }
        },
    )
    .await
    .unwrap();
    let completion = wait(admission).await;
    if stage == CompactionHookStage::SessionStartCompact {
        assert_eq!(completion, TaskCompletion::Completed);
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
        assert_eq!(authority.calls.load(Ordering::SeqCst), 0);
        assert!(!authority.root.join("input.json").exists());
        let restored = AgentSessionService::from_app_store(&f.store).unwrap();
        let pending = restored
            .active_context_checkpoint(f.record.conversation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.turn_id, f.record.turn_id);
        assert!(
            restored
                .compact_hook_contexts(f.record.conversation_id)
                .await
                .unwrap()
                .is_empty()
        );
        followup(&f, &authority.root, &config, restored, consent).await;
        return;
    }
    assert_eq!(completion, TaskCompletion::Canceled);
    let installed = stage != CompactionHookStage::PreCompact;
    assert_eq!(
        transport.requests.lock().unwrap().len(),
        usize::from(installed)
    );
    let checkpoint = f
        .sessions
        .active_context_checkpoint(f.record.conversation_id)
        .await
        .unwrap();
    assert_eq!(checkpoint.is_some(), installed);
    if stage == CompactionHookStage::PostCompact {
        crate::agent::compaction_commands::test_fixtures::export_service(
            "post-canceled",
            &f.sessions,
            f.record.conversation_id,
        )
        .await;
    }
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(authority.root.join("input.json")).unwrap()).unwrap();
    assert_eq!(input["session_id"], f.record.conversation_id.to_string());
    assert_eq!(input["turn_id"], f.record.turn_id.to_string());
    assert_eq!(input["hook_event_name"], event_name);
    assert_eq!(input["trigger"], "manual");
    assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
    let retry = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("receipt retry must not prepare or execute hook") },
    )
    .await
    .unwrap();
    assert!(matches!(retry, SubmissionAdmission::AlreadyStarted { .. }));
    assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
    assert!(
        f.sessions
            .active_context_checkpoint(f.record.conversation_id)
            .await
            .unwrap()
            == checkpoint
    );
    let restored = AgentSessionService::from_app_store(&f.store).unwrap();
    let contexts = restored
        .compact_hook_contexts(f.record.conversation_id)
        .await
        .unwrap();
    assert!(contexts.is_empty());
}

async fn followup(
    f: &Fixture,
    root: &std::path::Path,
    config: &str,
    service: AgentSessionService,
    consent: Option<Arc<SqliteHookConsent>>,
) {
    let store = Arc::new(SqliteSubmissionStore::new(
        f.store.shared_agent_connection(),
    ));
    let mut input = f.record.input.clone();
    input.text = "Follow the compacted context".into();
    let record = store
        .put(f.record.conversation_id, Uuid::new_v4(), input)
        .await
        .unwrap();
    let authority = Arc::new(Authority {
        root: root.into(),
        record: record.clone(),
        revision: compaction_hook_config::parse(config)
            .unwrap()
            .revision()
            .into(),
        stage: CompactionHookStage::SessionStartCompact,
        calls: AtomicUsize::new(0),
    });
    let factory = configured_factory(root.into(), authority.clone(), consent);
    let models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(root.join("unused-followup-credentials.json")),
    )
    .with_compaction_hooks(factory);
    let hooks = models
        .prepare_compaction_hooks(AgentHost::Notes, &record)
        .await
        .unwrap();
    let transport = transport(f, false);
    let profile = execution_profile_for_host(
        &record.input.model,
        execution_mode_for_model(
            &record.input.model,
            record.input.reasoning_effort.as_deref(),
        ),
        AgentHost::Notes,
    )
    .unwrap();
    let execution = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: hooks,
        model: record.input.model.clone(),
        compaction_strategy: Some(profile.compaction_strategy(transport.clone()).unwrap()),
        transport: transport.clone(),
        host_tools: OwnedToolRegistration::new(
            vec![],
            Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
        ),
        system_instructions: crate::agent::host_instructions::NOTES.into(),
        limits: profile.limits,
        max_output_tokens: profile.max_output_tokens,
        max_input_bytes: profile.max_input_bytes,
    });
    let service = Arc::new(service);
    let admission = execution
        .clone()
        .start(
            service.clone(),
            store.clone(),
            ConversationId::from_uuid(record.conversation_id),
            record.request_id,
        )
        .await
        .unwrap();
    assert_eq!(wait(admission).await, TaskCompletion::Canceled);
    assert!(transport.requests.lock().unwrap().is_empty());
    assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
    let request: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("input.json")).unwrap()).unwrap();
    assert_eq!(request["turn_id"], record.turn_id.to_string());
    assert_eq!(request["session_id"], record.conversation_id.to_string());
    assert_eq!(request["hook_event_name"], "SessionStart");
    assert_eq!(request["source"], "compact");
    assert!(request.get("trigger").is_none());
    let restored = AgentSessionService::from_app_store(&f.store).unwrap();
    let contexts = restored
        .compact_hook_contexts(record.conversation_id)
        .await
        .unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].turn_id, record.turn_id);
    assert_eq!(contexts[0].contexts, ["restored configured context"]);
    assert!(
        restored
            .take_compact_session_start(record.conversation_id)
            .await
            .unwrap()
            .is_none()
    );
    std::fs::remove_file(root.join("input.json")).unwrap();
    let mut input = record.input.clone();
    input.text = "Continue after the hook stopped".into();
    let next = store
        .put(record.conversation_id, Uuid::new_v4(), input)
        .await
        .unwrap();
    let next_authority = Arc::new(Authority {
        root: root.into(),
        record: next.clone(),
        revision: authority.revision.clone(),
        stage: CompactionHookStage::SessionStartCompact,
        calls: AtomicUsize::new(0),
    });
    let factory = Arc::new(
        ConfiguredHookFactory::new(root.into(), next_authority.clone(), Arc::new(Observer))
            .unwrap(),
    );
    let models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(root.join("unused-next-credentials.json")),
    )
    .with_compaction_hooks(factory);
    let hooks = models
        .prepare_compaction_hooks(AgentHost::Notes, &next)
        .await
        .unwrap();
    let continued = Arc::new(AgentExecution {
        #[cfg(target_os = "macos")]
        stop_hooks: None,
        compaction_hooks: hooks,
        model: next.input.model.clone(),
        transport: transport.clone(),
        compaction_strategy: execution.compaction_strategy.clone(),
        host_tools: OwnedToolRegistration::new(
            vec![],
            Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
        ),
        system_instructions: execution.system_instructions.clone(),
        limits: execution.limits,
        max_output_tokens: execution.max_output_tokens,
        max_input_bytes: execution.max_input_bytes,
    });
    let admission = continued
        .start(
            Arc::new(restored),
            store,
            ConversationId::from_uuid(next.conversation_id),
            next.request_id,
        )
        .await
        .unwrap();
    assert_eq!(wait(admission).await, TaskCompletion::Completed);
    assert_eq!(next_authority.calls.load(Ordering::SeqCst), 0);
    assert!(!root.join("input.json").exists());
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].messages.iter().filter(|message| matches!(message, ModelMessage::Developer(text) if text == "restored configured context")).count(), 1);
}
