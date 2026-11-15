use super::*;
use crate::agent::compaction_hook_factory::*;
use research_agent_core::context::compaction_hooks::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct StopHook;
impl CompactionHooks for StopHook {
    fn notify(&self, _: &CompactionHookRequest, _: CompactionHookNotification) {}
    fn run(&self, request: CompactionHookRequest) -> CompactionHookFuture<'_> {
        assert_eq!(request.stage, CompactionHookStage::PreCompact);
        Box::pin(async { CompactionHookOutcome::Stop })
    }
}
struct Factory {
    record: SubmissionRecord,
    calls: AtomicUsize,
    denied: bool,
}
impl CompactionHookFactory for Factory {
    fn prepare<'a>(
        &'a self,
        host: AgentHost,
        record: &'a SubmissionRecord,
    ) -> ModelFuture<'a, PreparedCompactionHooks> {
        assert!(matches!(host, AgentHost::Notes));
        assert_eq!(record, &self.record);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.denied {
                Err(ModelError::Authentication)
            } else {
                Ok(Some(Arc::new(StopHook) as Arc<dyn CompactionHooks>))
            }
        })
    }
}
fn resolver(f: &Fixture, factory: Arc<Factory>) -> AgentModelResolver {
    AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(f._root.path().join("hook-credentials.json")),
    )
    .with_compaction_hooks(factory)
}

#[tokio::test]
async fn prepared_manual_hook_stops_before_inference_and_retry_does_not_prepare_again() {
    let f = fixture("/compact").await;
    let factory = Arc::new(Factory {
        record: f.record.clone(),
        calls: AtomicUsize::new(0),
        denied: false,
    });
    let models = resolver(&f, factory.clone());
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
                Ok((transport as Arc<dyn ModelTransport>, hooks, empty_tools()))
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(wait(admission).await, TaskCompletion::Canceled);
    assert!(transport.requests.lock().unwrap().is_empty());
    assert!(
        f.sessions
            .active_context_checkpoint(f.record.conversation_id)
            .await
            .unwrap()
            .is_none()
    );
    let retry = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("retry must not reload hook consent") },
    )
    .await
    .unwrap();
    assert!(matches!(retry, SubmissionAdmission::AlreadyStarted { .. }));
    assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn denied_hook_factory_prevents_task_admission_and_model_io() {
    let f = fixture("/compact").await;
    let factory = Arc::new(Factory {
        record: f.record.clone(),
        calls: AtomicUsize::new(0),
        denied: true,
    });
    let models = resolver(&f, factory.clone());
    let result = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |record, _| async move {
            let _ = models
                .prepare_compaction_hooks(AgentHost::Notes, &record)
                .await
                .map_err(|_| Error::ModelUnavailable)?;
            panic!("denied preparation must not create a transport")
        },
    )
    .await;
    assert!(matches!(result, Err(Error::ModelUnavailable)));
    let history = f
        .sessions
        .open(ConversationId::from_uuid(f.record.conversation_id))
        .await
        .unwrap();
    assert!(history.turn(f.record.turn_id).is_none());
    assert_eq!(factory.calls.load(Ordering::SeqCst), 1);
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn default_native_factory_stops_real_manual_task_and_duplicate_keeps_original_admission() {
    use crate::agent::{compaction_hook_commands as settings, host_execution};
    let f = fixture("/compact").await;
    {
        let db = f.store.connection().unwrap();
        let page_id = match &f.request.host {
            Host::Note { page_id } => *page_id,
            _ => panic!("note fixture"),
        };
        db.execute("insert into agent_host_shell_grants(conversation_id,host_kind,resource_id,revision) values (?1,'note',?2,'native-r1')",
            rusqlite::params![f.record.conversation_id.to_string(), page_id.to_string()]).unwrap();
        host_execution::freeze_grant_sql(&db, &f.record).unwrap();
    }
    // The fixture opens the canonical native app root; no selected hook path
    // crosses the IPC or model boundary.
    let config = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("hooks.toml"), "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='pre_compact'\nenabled=true\nprogram='/bin/sh'\narguments=['-c', 'cat >/dev/null; touch admitted-hook; printf \"{\\\"continue\\\":false}\"']\ntimeout_ms=5000\nmax_output_bytes=1024\n").unwrap();
    let settings = Arc::new(settings::HookConsentService::from_app_store(&f.store));
    let scope = settings::Scope {
        host: settings::Host::Notes,
        model: f.record.input.model.clone(),
    };
    let review = settings.review(scope.clone()).unwrap().handlers.remove(0);
    settings
        .approve(settings::ApproveRequest {
            scope,
            handler_id: review.handler_id,
            expected_revision: review.revision,
            fingerprint: review.fingerprint,
        })
        .unwrap();
    let models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(f._root.path().join("unused-native-credentials.json")),
    )
    .with_native_compaction_hooks(settings);
    let transport = transport(&f, false);
    let work = f
        .store
        .agent_workspace_directory()
        .join(f.record.turn_id.to_string())
        .join("work");
    let admission = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |record, _| {
            let models = &models;
            let store = f.store.clone();
            let sessions = f.sessions.clone();
            let transport = transport.clone();
            async move {
                let registered = host_execution::registration_with_hooks(
                    store,
                    record.clone(),
                    sessions,
                    OwnedToolRegistration::new(
                        vec![],
                        Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
                    ),
                )
                .await
                .map_err(|_| Error::ModelUnavailable)?;
                let hooks = models
                    .prepare_compaction_hooks_for_profile(
                        AgentHost::Notes,
                        &record,
                        &crate::agent::execution_profile::execution_profile(
                            &record.input.model,
                            crate::agent::execution_profile::AgentExecutionMode::DeepSeekNonThinking,
                        ).map_err(|_| Error::ModelUnavailable)?,
                        registered.workspace,
                    )
                    .await
                    .map_err(|_| Error::ModelUnavailable)?;
                Ok((transport as Arc<dyn ModelTransport>, hooks, empty_tools()))
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(wait(admission).await, TaskCompletion::Canceled);
    assert!(work.join("admitted-hook").exists());
    assert!(transport.requests.lock().unwrap().is_empty());
    assert!(
        f.sessions
            .active_context_checkpoint(f.record.conversation_id)
            .await
            .unwrap()
            .is_none()
    );
    let retry = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("duplicate cannot reprepare native workspace or run the hook") },
    )
    .await
    .unwrap();
    assert!(matches!(retry, SubmissionAdmission::AlreadyStarted { .. }));
}
