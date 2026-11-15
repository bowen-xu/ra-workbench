use super::*;
const MAX_SHELL_POLLS: usize = 7;
struct ShellTransport {
    calls: AtomicUsize,
    reached: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    continue_file: std::path::PathBuf,
}
impl ModelTransport for ShellTransport {
    fn capabilities(&self) -> ModelCapabilities {
        Transport(AtomicUsize::new(0)).capabilities()
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert!(
                request
                    .tools
                    .iter()
                    .any(|tool| tool.name == crate::agent::shell_tools::EXEC)
            );
            let mut events = vec![ModelStreamEvent::Started {
                response_id: "shell-fixture".into(),
                actual_model: request.model,
            }];
            if call == 0 {
                events.extend([
                    ModelStreamEvent::ToolCallStarted {call_id:"edit-copy".into(),name:crate::agent::shell_tools::EXEC.into()},
                    ModelStreamEvent::ToolArgumentsDelta {call_id:"edit-copy".into(),delta:serde_json::json!({"command":"while [ ! -f .shell-continue ]; do sleep 0.01; done; rm .shell-continue; printf agent-draft > main.tex; printf appendix-draft > appendix.tex; printf executed-copy","yield_ms":0}).to_string()},
                    ModelStreamEvent::ToolCallCompleted {call_id:"edit-copy".into()},
                    ModelStreamEvent::Completed { reason: FinishReason::ToolCalls, end_turn: None },
                ]);
            } else {
                let output = request
                    .messages
                    .iter()
                    .rev()
                    .find_map(|message| match message {
                        ModelMessage::ToolResult { content, .. } => Some(content),
                        _ => None,
                    })
                    .expect("shell continuation must retain its tool result");
                let result: serde_json::Value = serde_json::from_str(output).unwrap();
                if result["state"] == "running" {
                    assert!(
                        call <= MAX_SHELL_POLLS,
                        "shell did not finish after bounded polls: {output}"
                    );
                    // Exercise almost the whole poll budget before releasing
                    // the real process. Leave one extra poll for final draining.
                    if call == MAX_SHELL_POLLS - 1 {
                        std::fs::write(&self.continue_file, b"continue").unwrap();
                    }
                    let call_id = format!("poll-copy-{call}");
                    events.extend([
                        ModelStreamEvent::ToolCallStarted {
                            call_id: call_id.clone(),
                            name: crate::agent::shell_tools::PROCESS.into(),
                        },
                        ModelStreamEvent::ToolArgumentsDelta {
                            call_id: call_id.clone(),
                            delta: serde_json::json!({"action":"poll","process_id":result["processId"],"yield_ms":if call < MAX_SHELL_POLLS - 1 { 0 } else { 1000 }}).to_string(),
                        },
                        ModelStreamEvent::ToolCallCompleted { call_id },
                        ModelStreamEvent::Completed { reason: FinishReason::ToolCalls, end_turn: None },
                    ]);
                    return Ok(Box::new(Stream(events.into())) as Box<dyn ModelStream>);
                }
                assert_eq!(result["state"], "finished", "shell result: {output}");
                assert_eq!(result["exitCode"], 0, "shell result: {output}");
                assert_eq!(result["stdout"]["head"], "executed-copy");
                self.reached.notify_one();
                self.resume.notified().await;
                events.extend([
                    ModelStreamEvent::TextDelta("Working copy edited".into()),
                    ModelStreamEvent::Completed {
                        reason: FinishReason::Stop,
                        end_turn: None,
                    },
                ]);
            }
            Ok(Box::new(Stream(events.into())) as Box<dyn ModelStream>)
        })
    }
}
#[tokio::test]
async fn approved_shell_runs_in_real_writing_loop_and_duplicate_start_does_not_reexecute() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    // This legacy shell script explicitly reviews and publishes its proposal
    // below. Freeze that manual-publication contract for this request while
    // retaining the latest managed schema and the production v2 default.
    f.store.connection().unwrap().execute("insert into agent_text_tool_contracts(conversation_id,request_id,version) values(?1,?2,1)",
        rusqlite::params![record.conversation_id.to_string(),record.request_id.to_string()]).unwrap();
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
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "shell-loop".into())
            .unwrap(),
    )));
    let unapproved = super::super::super::tools::for_submission(
        f.store.clone(),
        record.clone(),
        sessions.clone(),
    )
    .await
    .unwrap();
    assert!(
        !unapproved
            .definitions()
            .iter()
            .any(|tool| tool.name == crate::agent::shell_tools::EXEC)
    );
    let approved = service.approve_workspace(prepared).await.unwrap();
    let lease = approved.binding.lease_id;
    let continue_file = approved
        .lease
        .working_directory()
        .unwrap()
        .join(".shell-continue");
    drop(approved);
    let transport = Arc::new(ShellTransport {
        calls: AtomicUsize::new(0),
        reached: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
        continue_file,
    });
    let mut execution_profile = profile(record.input.model.clone());
    // Reserve the exec request, every bounded poll, and the final text request.
    // A slower process must not exhaust the generic three-request fixture.
    execution_profile.limits.model_calls = Some(MAX_SHELL_POLLS + 2);
    execution_profile.limits.tool_calls = Some(MAX_SHELL_POLLS + 1);
    // Tool schemas and cumulative process results also count toward each
    // request. The small generic fixture cannot fit this many continuations.
    execution_profile.max_input_bytes = 100_000;
    let admission = service
        .start_transport(
            sessions.clone(),
            record.clone(),
            execution_profile.clone(),
            transport.clone(),
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!()
    };
    tokio::time::timeout(
        // This integration assertion is about continuation and ownership, not
        // scheduler latency while other tests run full database migrations.
        std::time::Duration::from_secs(120),
        async {
            tokio::select! {
                () = transport.reached.notified() => {}
                completion = owned.task.wait() => {
                    let failure = f.store.connection().unwrap().query_row(
                        "select json_extract(payload_json,'$.payload.message') from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='error' order by seq desc limit 1",
                        [f.conversation.to_string()], |row| row.get::<_, String>(0),
                    ).ok();
                    panic!("shell turn ended before final continuation after {} requests: {completion:?}, {failure:?}", transport.calls.load(Ordering::SeqCst));
                }
            }
        },
    )
    .await
    .unwrap_or_else(|_| {
        panic!(
            "shell continuation was not reached after {} model requests",
            transport.calls.load(Ordering::SeqCst)
        )
    });
    let calls_before_retry = transport.calls.load(Ordering::SeqCst);
    assert!(calls_before_retry > MAX_SHELL_POLLS);
    let running_retry = service
        .start_transport(
            sessions.clone(),
            record.clone(),
            execution_profile.clone(),
            transport.clone(),
        )
        .await
        .unwrap();
    assert!(matches!(
        running_retry,
        SubmissionAdmission::AlreadyStarted { outcome: None, .. }
    ));
    transport.resume.notify_one();
    assert_eq!(owned.task.wait().await, TaskCompletion::Completed);
    assert_eq!(transport.calls.load(Ordering::SeqCst), calls_before_retry);
    assert_eq!(
        std::fs::read(
            f.store
                .agent_workspace_directory()
                .join(lease.to_string())
                .join("work/main.tex")
        )
        .unwrap(),
        b"agent-draft"
    );
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"Frozen manuscript"
    );

    // The model-created workspace edit must use the same review and ChangeSet
    // boundary as a UI-originated proposal; no direct source write is allowed.
    std::fs::create_dir_all(f.store.content_directory()).unwrap();
    let review = service
        .review_workspace(receipt, record.context_snapshot_id)
        .await
        .unwrap();
    assert_eq!(review.changes.len(), 2);
    assert!(
        review
            .changes
            .iter()
            .any(|change| change.path == "main.tex")
    );
    assert!(
        review
            .changes
            .iter()
            .any(|change| change.path == "appendix.tex")
    );
    let proposal = service
        .publish_workspace_changes(review.binding.clone(), review.changes)
        .await
        .unwrap();
    let operation = Uuid::new_v4();
    let admission = service
        .begin_proposal_application(
            f.conversation,
            proposal,
            operation,
            crate::agent::writing_workspace::WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        admission,
        crate::agent::repository::proposal_applications::ApplicationAdmission::Created(id)
            if id == operation
    ));
    let applied = service
        .apply_disk_proposal_file(
            crate::agent::repository::proposal_applications::ApplicationScope {
                conversation: f.conversation,
                manuscript: f.manuscript,
                proposal,
                operation,
            },
            "main.tex".into(),
        )
        .await
        .unwrap();
    assert!(applied.files.iter().all(|file| {
        file.path != "main.tex"
            || file.phase
                == crate::agent::repository::proposal_applications::ApplicationPhase::Acknowledged
    }));
    let applied = service
        .apply_added_proposal_file(
            crate::agent::repository::proposal_applications::ApplicationScope {
                conversation: f.conversation,
                manuscript: f.manuscript,
                proposal,
                operation,
            },
            "appendix.tex".into(),
        )
        .await
        .unwrap();
    assert!(applied.files.iter().all(|file| {
        file.path != "appendix.tex"
            || file.phase
                == crate::agent::repository::proposal_applications::ApplicationPhase::Acknowledged
    }));
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"agent-draft"
    );
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/appendix.tex")
        )
        .unwrap(),
        b"appendix-draft"
    );
    let retry = service
        .start_transport(
            sessions.clone(),
            record.clone(),
            profile(record.input.model.clone()),
            transport.clone(),
        )
        .await
        .unwrap();
    assert!(matches!(retry, SubmissionAdmission::AlreadyStarted { .. }));
    assert_eq!(transport.calls.load(Ordering::SeqCst), calls_before_retry);
}

#[tokio::test]
async fn agent_workspace_edit_flows_through_review_publication_and_disk_acceptance() {
    let f = fixture(true);
    std::fs::create_dir_all(f.store.content_directory()).unwrap();
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
    let binding = prepared.binding.clone();
    let lease = prepared.lease.manifest().identity.lease_id;
    let approved = service.approve_workspace(prepared).await.unwrap();
    std::fs::write(
        approved.lease.working_directory().unwrap().join("main.tex"),
        b"agent-draft",
    )
    .unwrap();
    drop(approved);

    let review = service
        .review_workspace(receipt, record.context_snapshot_id)
        .await
        .unwrap();
    assert_eq!(review.binding.lease_id, lease);
    assert_eq!(review.changes.len(), 1);
    assert_eq!(review.changes[0].path, "main.tex");

    let proposal = service
        .publish_workspace_changes(review.binding.clone(), review.changes.clone())
        .await
        .unwrap();
    let operation = Uuid::new_v4();
    let admission = service
        .begin_proposal_application(
            f.conversation,
            proposal,
            operation,
            crate::agent::writing_workspace::WritingWorkspaceRequest {
                manuscript_id: f.manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        admission,
        crate::agent::repository::proposal_applications::ApplicationAdmission::Created(id)
            if id == operation
    ));
    let scope = crate::agent::repository::proposal_applications::ApplicationScope {
        conversation: f.conversation,
        manuscript: f.manuscript,
        proposal,
        operation,
    };
    let applied = service
        .apply_disk_proposal_file(scope, "main.tex".into())
        .await
        .unwrap();
    assert_eq!(
        applied
            .files
            .iter()
            .find(|file| file.path == "main.tex")
            .map(|file| file.phase),
        Some(crate::agent::repository::proposal_applications::ApplicationPhase::Acknowledged)
    );
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"agent-draft"
    );
    assert_eq!(binding.manuscript_id, f.manuscript);
}
