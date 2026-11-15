use super::*;
use research_agent_core::submission::SubmissionInput;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Observer;
impl HookHandlerObserver for Observer {
    fn notify(&self, _: Uuid, _: &CompactionHookRequest, _: CompactionHookNotification) {}
}
struct Authority {
    root: PathBuf,
    record: SubmissionRecord,
    revision: String,
    source_allowed: AtomicBool,
    invocation_allowed: AtomicBool,
    grants: AtomicUsize,
    checks: AtomicUsize,
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
        self.source_allowed
            .load(Ordering::SeqCst)
            .then_some(())
            .ok_or(HookConfigError)
    }
    fn authorize_command(
        &self,
        root: &InstructionRootIdentity<'_>,
        hook: &ConfiguredHook,
        revision: &str,
        _: AgentHost,
        record: &SubmissionRecord,
    ) -> Result<AuthorizedHookCommand, HookConfigError> {
        assert_eq!(root.canonical_path, self.root);
        assert_eq!(record, &self.record);
        self.grants.fetch_add(1, Ordering::SeqCst);
        if revision != self.revision {
            return Err(HookConfigError);
        }
        Ok(AuthorizedHookCommand {
            process: hook.candidate()?,
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
        self.checks.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.model, self.record.input.model);
        assert_eq!(request.turn_id, self.record.turn_id);
        assert_eq!(request.conversation_id, self.record.conversation_id);
        if revision != self.revision || !self.invocation_allowed.load(Ordering::SeqCst) {
            return Err(HookConfigError);
        }
        Ok(())
    }
}
fn record() -> SubmissionRecord {
    SubmissionRecord {
        conversation_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
        input: SubmissionInput {
            remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
            text: "/compact".into(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "model".into(),
                revision: Some("1".into()),
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![],
        },
    }
}
fn request(record: &SubmissionRecord) -> CompactionHookRequest {
    CompactionHookRequest {
        operation_id: Uuid::new_v4(),
        conversation_id: record.conversation_id,
        turn_id: record.turn_id,
        stage: CompactionHookStage::SessionStartCompact,
        phase: research_agent_core::context::compaction_strategy::CompactionPhase::Manual,
        model: record.input.model.clone(),
    }
}
fn text(root: &std::path::Path) -> String {
    format!(
        "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='session_start_compact'\nenabled=true\nprogram='/bin/sh'\ndirectory='{}'\narguments=['-c','cat >/dev/null; touch executed; printf frozen-context']\ntimeout_ms=2000\nmax_output_bytes=1024\n",
        root.display()
    )
}

#[tokio::test]
async fn configured_factory_freezes_authorized_source_and_preserves_live_revocation() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("agent")).unwrap();
    let text = text(&root);
    let path = root.join("agent/hooks.toml");
    std::fs::write(&path, &text).unwrap();
    let record = record();
    let authority = Arc::new(Authority {
        root: root.clone(),
        record: record.clone(),
        revision: parse(&text).unwrap().revision().into(),
        source_allowed: AtomicBool::new(true),
        invocation_allowed: AtomicBool::new(true),
        grants: AtomicUsize::new(0),
        checks: AtomicUsize::new(0),
    });
    let factory =
        ConfiguredHookFactory::new(root.clone(), authority.clone(), Arc::new(Observer)).unwrap();
    let hooks = factory
        .prepare(AgentHost::Notes, &record)
        .await
        .unwrap()
        .unwrap();
    assert!(!root.join("executed").exists());
    for identity in 0..3 {
        let mut foreign = request(&record);
        match identity {
            0 => foreign.conversation_id = Uuid::new_v4(),
            1 => foreign.turn_id = Uuid::new_v4(),
            _ => foreign.model.revision = Some("foreign-revision".into()),
        }
        assert!(
            hooks
                .run_with_context(foreign)
                .await
                .additional_contexts
                .is_empty()
        );
        assert!(!root.join("executed").exists());
    }
    assert_eq!(authority.checks.load(Ordering::SeqCst), 0);
    std::fs::write(&path, format!("{text}# changed version\n")).unwrap();
    assert_eq!(
        hooks
            .run_with_context(request(&record))
            .await
            .additional_contexts,
        ["frozen-context"]
    );
    std::fs::remove_file(root.join("executed")).unwrap();
    authority.invocation_allowed.store(false, Ordering::SeqCst);
    assert!(
        hooks
            .run_with_context(request(&record))
            .await
            .additional_contexts
            .is_empty()
    );
    assert!(!root.join("executed").exists());
    assert!(factory.prepare(AgentHost::Notes, &record).await.is_err());
    assert_eq!(authority.grants.load(Ordering::SeqCst), 2);
    assert_eq!(authority.checks.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn source_denial_precedes_config_parsing_and_missing_config_does_not_grant_commands() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let record = record();
    let authority = Arc::new(Authority {
        root: root.clone(),
        record: record.clone(),
        revision: String::new(),
        source_allowed: AtomicBool::new(true),
        invocation_allowed: AtomicBool::new(true),
        grants: AtomicUsize::new(0),
        checks: AtomicUsize::new(0),
    });
    let factory =
        ConfiguredHookFactory::new(root.clone(), authority.clone(), Arc::new(Observer)).unwrap();
    assert!(
        factory
            .prepare(AgentHost::Notes, &record)
            .await
            .unwrap()
            .is_none()
    );
    std::fs::create_dir(root.join("agent")).unwrap();
    std::fs::write(root.join("agent/hooks.toml"), "malformed config").unwrap();
    authority.source_allowed.store(false, Ordering::SeqCst);
    assert!(factory.prepare(AgentHost::Notes, &record).await.is_err());
    assert_eq!(authority.grants.load(Ordering::SeqCst), 0);
}

#[path = "tests/cancel.rs"]
mod cancel;
#[path = "tests/consent.rs"]
mod consent;
#[cfg(target_os = "macos")]
#[path = "tests/sandbox.rs"]
mod sandbox;
#[path = "tests/sources.rs"]
mod sources;
