//! A real nonzero command exit is failed hook evidence, not a compaction veto.
use super::*;

#[tokio::test]
async fn configured_postcompact_exit_seven_records_handler_failure_and_completes_compaction() {
    let f = fixture("/compact").await;
    let root = f._root.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("agent")).unwrap();
    let config = format!(
        "schema_version=1\n[[handlers]]\nid='00000000-0000-0000-0000-000000000012'\nstage='post_compact'\nenabled=true\nprogram='/bin/sh'\ndirectory='{}'\narguments=['-c', 'cat > input.json; exit 7']\ntimeout_ms=2000\nmax_output_bytes=1024\n",
        root.display()
    );
    std::fs::write(root.join("agent/hooks.toml"), &config).unwrap();
    let authority = Arc::new(Authority {
        root: root.clone(),
        record: f.record.clone(),
        stage: CompactionHookStage::PostCompact,
        revision: compaction_hook_config::parse(&config)
            .unwrap()
            .revision()
            .into(),
        calls: AtomicUsize::new(0),
    });
    let models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(root.join("unused-credentials.json")),
    )
    .with_compaction_hooks(configured_factory(root, authority.clone(), None));
    let transport = transport(&f, false);
    let admitted = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |record, _| {
            let models = &models;
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
    assert_eq!(wait(admitted).await, TaskCompletion::Completed);
    assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read(authority.root.join("input.json")).unwrap()).unwrap();
    assert_eq!(input["hook_event_name"], "PostCompact");
    let (aggregate, handler): (String, String) = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select a.outcome,h.outcome from agent_compaction_hook_audit a join agent_compaction_hook_handlers h on h.audit_seq=a.seq where a.stage='post_compact' and a.notification='completed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    // Fixed Codex treats a failed handler as non-vetoing, while retaining its
    // actual status separately from the aggregate continuation decision.
    assert_eq!(aggregate, "continue");
    assert_eq!(handler, "failed");
    let id = ConversationId::from_uuid(f.record.conversation_id);
    let history = f.sessions.open(id).await.unwrap();
    let turn = history.turn(f.record.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    assert!(turn.failure.is_none());
    assert_eq!(turn.items.len(), 1);
    assert!(
        turn.items
            .values()
            .all(|item| item.completed && !item.interrupted)
    );
    let installed = f
        .sessions
        .active_context_checkpoint(f.record.conversation_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(installed.turn_id, f.record.turn_id);
    let retry = start_prepared(
        &f.store,
        f.sessions.clone(),
        f.request.clone(),
        |_, _| async { panic!("same receipt must not execute failed command again") },
    )
    .await
    .unwrap();
    assert!(matches!(
        retry,
        SubmissionAdmission::AlreadyStarted {
            outcome: Some(TurnOutcome::Completed),
            ..
        }
    ));
    assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
    let reopened = AgentSessionService::from_app_store(&f.store).unwrap();
    assert_eq!(reopened.open(id).await.unwrap(), history);
    assert!(
        reopened
            .active_context_checkpoint(f.record.conversation_id)
            .await
            .unwrap()
            .unwrap()
            == installed
    );
}
