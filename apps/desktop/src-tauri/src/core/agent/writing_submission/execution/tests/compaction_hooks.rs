use super::*;
use crate::agent::{
    compaction_hook_commands::{ApproveRequest, HookConsentService, Host, Scope},
    model::AgentHost,
};
use research_agent_core::context::{
    compaction_hooks::{CompactionHookOutcome, CompactionHookRequest, CompactionHookStage},
    compaction_strategy::CompactionPhase,
};

struct RecordingTransport {
    inner: Transport,
    requests: std::sync::Mutex<Vec<ModelRequest>>,
    sessions: Arc<AgentSessionService>,
    conversation: Uuid,
    turn: Uuid,
    expect_feedback: bool,
}
impl ModelTransport for RecordingTransport {
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            if self.expect_feedback && self.inner.0.load(Ordering::SeqCst) > 0 {
                let feedback = request.messages.iter().find_map(|message| match message {
                    ModelMessage::User { source_item_id: Some(id), content } if content.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("revise answer") && text.contains("<hook_prompt"))) => Some(*id),
                    _ => None,
                }).expect("sourced hook feedback in second model request");
                let projection = self
                    .sessions
                    .open(ConversationId::from_uuid(self.conversation))
                    .await
                    .map_err(|_| ModelError::Unavailable)?;
                assert!(matches!(
                    projection.turn(self.turn).unwrap().items[&feedback].kind,
                    research_agent_core::protocol::ItemKind::HookPrompt { .. }
                ));
            }
            self.requests.lock().unwrap().push(request.clone());
            self.inner.start(request).await
        })
    }
}

#[tokio::test]
async fn writing_task_runs_authorized_stop_hook_without_resampling() {
    run_writing_stop(
        "cat >/dev/null; printf '{\"continue\":false}'",
        1,
        false,
        false,
    )
    .await;
}

#[tokio::test]
async fn writing_stop_feedback_is_committed_before_continuation() {
    run_writing_stop("input=$(cat); case \"$input\" in *'\"stop_hook_active\":true'*) printf '{\"continue\":false}' ;; *) printf 'revise answer' >&2; exit 2 ;; esac", 2, true, false).await;
}

#[tokio::test]
async fn writing_stop_without_changes_finishes_without_delivery_or_resampling() {
    run_writing_stop("cat >/dev/null", 1, false, false).await;
}

#[tokio::test]
async fn writing_stop_after_delivery_records_unavailable_workspace() {
    run_writing_stop("cat >/dev/null; printf modified > main.tex", 2, false, true).await;
}

async fn run_writing_stop(
    command: &str,
    expected_calls: usize,
    expect_feedback: bool,
    expect_failed_hook: bool,
) {
    let mut f = super::super::super::tests::managed_fixture_at(
        true,
        tempfile::Builder::new()
            .prefix("writing-stop-hook-")
            .tempdir_in("/private/tmp")
            .unwrap(),
    );
    std::fs::create_dir_all(f.store.content_directory()).unwrap();
    let owner = Arc::new(std::sync::OnceLock::<
        std::sync::Weak<crate::agent::writing_delivery_tool::DeliveryTools>,
    >::new());
    let receiver = owner.clone();
    let deliveries = Arc::new(AtomicUsize::new(0));
    let observed_deliveries = deliveries.clone();
    let delivery_store = f.store.clone();
    let runtime = tokio::runtime::Handle::current();
    let events = crate::runtime_events::RuntimeEvents::new(move |name, payload| {
        if name == "research-writing-file-delivery" {
            observed_deliveries.fetch_add(1, Ordering::SeqCst);
            let request: crate::agent::writing_delivery_tool::Request =
                serde_json::from_value(payload)
                    .map_err(|_| "invalid delivery request".to_owned())?;
            let owner = receiver
                .get()
                .and_then(std::sync::Weak::upgrade)
                .ok_or("delivery owner unavailable")?;
            let store = delivery_store.clone();
            runtime.spawn(async move {
                let service = WritingSubmissionService::from_app_store(&store);
                let review = service
                    .review_workspace(request.scope.receipt(), request.scope.context_snapshot_id)
                    .await
                    .unwrap();
                let proposal = service
                    .publish_workspace_changes(review.binding, review.changes)
                    .await
                    .unwrap();
                let operation = crate::agent::repository::proposal_applications::ApplicationScope {
                    conversation: request.scope.conversation_id,
                    manuscript: request.scope.manuscript_id,
                    proposal,
                    operation: Uuid::new_v4(),
                };
                service
                    .begin_proposal_application(
                        operation.conversation,
                        proposal,
                        operation.operation,
                        crate::agent::writing_workspace::WritingWorkspaceRequest {
                            manuscript_id: operation.manuscript,
                            buffers: vec![],
                        },
                    )
                    .await
                    .unwrap();
                service
                    .apply_disk_proposal_file(operation, "main.tex".into())
                    .await
                    .unwrap();
                owner
                    .report(crate::agent::writing_delivery_tool::ResultInput {
                        request,
                        complete: true,
                        message: "applied".into(),
                    })
                    .unwrap();
            });
        }
        Ok(())
    });
    let delivery = Arc::new(crate::agent::writing_delivery_tool::DeliveryTools::for_events(events));
    owner.set(Arc::downgrade(&delivery)).unwrap();
    f.store.set_test_writing_delivery_tools(delivery);
    let base = WritingSubmissionService::from_app_store(&f.store);
    let record = base
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: record.request_id,
        manuscript_id: f.manuscript,
    };
    let prepared = base
        .prepare_workspace(
            receipt,
            record.context_snapshot_id,
            crate::agent::writing_workspace::WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let approved = base.approve_workspace(prepared).await.unwrap();
    drop(approved);
    let config = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&config).unwrap();
    let command = toml::Value::String(command.into());
    std::fs::write(config.join("hooks.toml"), format!("schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='stop'\nenabled=true\nprogram='/bin/sh'\narguments=['-c', {command}]\ntimeout_ms=5000\nmax_output_bytes=1024\n")).unwrap();
    let settings = Arc::new(HookConsentService::from_app_store(&f.store));
    let scope = Scope {
        host: Host::Writing,
        model: record.input.model.clone(),
    };
    let review = settings.review(scope.clone()).unwrap().handlers.remove(0);
    settings
        .approve(ApproveRequest {
            scope,
            handler_id: review.handler_id,
            expected_revision: review.revision,
            fingerprint: review.fingerprint,
        })
        .unwrap();
    let models = f.models.clone().with_native_compaction_hooks(settings);
    let service = WritingSubmissionService::from_app_store(&f.store).with_model_resolver(models);
    let sessions = Arc::new(AgentSessionService::from_app_store(&f.store).unwrap());
    let transport = Arc::new(RecordingTransport {
        inner: Transport(AtomicUsize::new(0)),
        requests: std::sync::Mutex::new(vec![]),
        sessions: sessions.clone(),
        conversation: record.conversation_id,
        turn: record.turn_id,
        expect_feedback,
    });
    let admission = service
        .start_transport_with_manuscript(
            sessions.clone(),
            record.clone(),
            profile(record.input.model.clone()),
            transport.clone(),
            f.manuscript,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("task was not started")
    };
    assert_eq!(
        owned.task.wait().await,
        TaskCompletion::Completed,
        "{}",
        persisted_failure(&sessions, &record, &f.store).await
    );
    assert_eq!(transport.inner.0.load(Ordering::SeqCst), expected_calls);
    assert_eq!(
        deliveries.load(Ordering::SeqCst),
        usize::from(expect_failed_hook)
    );
    if expect_feedback {
        let feedback = {
            let requests = transport.requests.lock().unwrap();
            requests[1].messages.iter().find_map(|message| match message {
            ModelMessage::User { source_item_id: Some(id), content } if content.iter().any(|part| matches!(part, ContentPart::Text(text) if text.contains("revise answer") && text.contains("<hook_prompt"))) => Some(*id),
            _ => None,
        }).expect("sourced hook feedback in second model request")
        };
        let projection = sessions
            .open(ConversationId::from_uuid(record.conversation_id))
            .await
            .unwrap();
        assert!(matches!(
            projection.turn(record.turn_id).unwrap().items[&feedback].kind,
            research_agent_core::protocol::ItemKind::HookPrompt { .. }
        ));
    }
    let mut db = f.store.connection().unwrap();
    let count: i64 = db
        .query_row("select count(*) from agent_stop_hook_finishes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, expected_calls as i64);
    let failed: i64 = db.query_row("select count(*) from agent_stop_hook_results where json_extract(outcome_json,'$.status.kind')='failed'", [], |row| row.get(0)).unwrap();
    assert_eq!(failed, i64::from(expect_failed_hook));
    let (binding, manifest) = crate::agent::repository::execution_workspaces::read_for_review(
        &db,
        record.conversation_id,
        record.request_id,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        crate::agent::repository::execution_workspaces::authorize(&db, &binding, &manifest),
        Err(crate::agent::repository::execution_workspaces::Error::Denied)
    ));
    if expect_failed_hook {
        let diagnostic: String = db.query_row("select json_extract(outcome_json,'$.status.diagnostic') from agent_stop_hook_results where json_extract(outcome_json,'$.status.kind')='failed'", [], |row| row.get(0)).unwrap();
        assert_eq!(
            diagnostic,
            "stop hook authorization or workspace unavailable"
        );
        let tx = db.transaction().unwrap();
        assert!(matches!(
            crate::agent::repository::execution_workspaces::approve_in(&tx, &binding),
            Err(crate::agent::repository::execution_workspaces::Error::Denied)
        ));
        tx.rollback().unwrap();
    }
}

#[tokio::test]
async fn native_factory_reuses_writing_tools_lease_and_keeps_original_manuscript_unchanged() {
    // The native config reader intentionally rejects aliased/symlinked roots.
    // Use a canonical macOS temp container rather than the /var temp alias.
    let f = super::super::super::tests::managed_fixture_at(
        true,
        tempfile::Builder::new()
            .prefix("writing-native-hooks-")
            .tempdir_in("/private/tmp")
            .unwrap(),
    );
    let service = WritingSubmissionService::from_app_store(&f.store);
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    let receipt = WritingSubmissionReceipt {
        conversation_id: f.conversation,
        request_id: record.request_id,
        manuscript_id: f.manuscript,
    };
    let prepared = service
        .prepare_workspace(
            receipt,
            record.context_snapshot_id,
            crate::agent::writing_workspace::WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    let approved = service.approve_workspace(prepared).await.unwrap();
    let work = approved.lease.working_directory().unwrap();
    drop(approved);
    let config = f.store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("hooks.toml"),
        "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='pre_compact'\nenabled=true\nprogram='/bin/sh'\narguments=['-c', 'cat >/dev/null; printf hook-draft > main.tex; printf \"{\\\"continue\\\":false}\"']\ntimeout_ms=5000\nmax_output_bytes=1024\n").unwrap();
    let settings = Arc::new(HookConsentService::from_app_store(&f.store));
    let scope = Scope {
        host: Host::Writing,
        model: record.input.model.clone(),
    };
    let review = settings.review(scope.clone()).unwrap().handlers.remove(0);
    settings
        .approve(ApproveRequest {
            scope,
            handler_id: review.handler_id,
            expected_revision: review.revision,
            fingerprint: review.fingerprint,
        })
        .unwrap();
    let models = f.models.clone().with_native_compaction_hooks(settings);
    let sessions = Arc::new(AgentSessionService::from_app_store(&f.store).unwrap());
    sessions
        .processes()
        .begin_turn(record.conversation_id, record.turn_id)
        .unwrap();
    let tools = super::super::super::tools::for_submission(
        f.store.clone(),
        record.clone(),
        sessions.clone(),
    )
    .await
    .unwrap();
    let reader_authority = tools.hook_workspace().unwrap();
    reader_authority.authorize_artifact_read(&record).unwrap();
    let mut foreign = record.clone();
    foreign.input.model.revision = Some("foreign-reader-revision".into());
    assert!(reader_authority.authorize_artifact_read(&foreign).is_err());
    let hooks = models
        .prepare_compaction_hooks_for_profile(
            AgentHost::Writing,
            &record,
            &profile(record.input.model.clone()),
            tools.hook_workspace(),
        )
        .await
        .unwrap()
        .unwrap();
    // Repeated admission preparation retains another lazy handle, but never
    // competes for the already-owned lease before its task has been admitted.
    let duplicate = super::super::super::tools::for_submission(
        f.store.clone(),
        record.clone(),
        sessions.clone(),
    )
    .await
    .unwrap();
    assert!(duplicate.hook_workspace().is_some());
    let result = hooks
        .run(CompactionHookRequest {
            operation_id: Uuid::new_v4(),
            conversation_id: record.conversation_id,
            turn_id: record.turn_id,
            model: record.input.model.clone(),
            phase: CompactionPhase::Manual,
            stage: CompactionHookStage::PreCompact,
        })
        .await;
    assert_eq!(result, CompactionHookOutcome::Stop);
    assert_eq!(
        std::fs::read_to_string(work.join("main.tex")).unwrap(),
        "hook-draft"
    );
    let call = ToolCall {
        namespace: None,
        raw_arguments_json: None,
        call_id: "read-native-hook-draft".into(),
        name: crate::agent::shell_tools::EXEC.into(),
        arguments: serde_json::json!({"command":"cat main.tex","yield_ms":5000}),
    };
    let result = tools.authorize_and_execute(call.clone()).await.unwrap();
    assert!(
        !result.is_error && result.content.contains("hook-draft"),
        "{}",
        result.content
    );
    tools.result_committed(call).await.unwrap();
    let original = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/main.tex");
    assert_eq!(
        std::fs::read_to_string(original).unwrap(),
        "Frozen manuscript"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "delete from agent_execution_workspaces where conversation_id=?1 and request_id=?2",
            rusqlite::params![
                record.conversation_id.to_string(),
                record.request_id.to_string()
            ],
        )
        .unwrap();
    assert!(reader_authority.authorize_artifact_read(&record).is_err());
    sessions.processes().shutdown().await.unwrap();
}
