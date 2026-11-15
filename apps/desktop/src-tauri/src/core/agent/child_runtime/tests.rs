use super::*;
use research_agent_core::{
    conversation::{BindingKind, ConversationBinding, ConversationId},
    protocol::ModelIdentity,
    submission::{SubmissionInput, SubmissionStore},
};

#[tokio::test]
async fn failed_lineage_requests_cancel_but_keeps_started_child_owned_until_completion() {
    let root = tempfile::tempdir().unwrap();
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let child = Uuid::new_v4();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let owned = sessions
        .start_owned_turn(
            ConversationId::from_uuid(child),
            move |_, signal| async move {
                let _ = ready_tx.send(());
                signal.cancelled().await;
                let _ = cancel_tx.send(());
                let _ = release_rx.await;
                Ok(())
            },
        )
        .unwrap();
    let turn = owned.turn_id;
    let receipt = owned.task.clone();
    ready_rx.await.unwrap();
    assert!(
        track_started(
            &store,
            &sessions,
            ConversationId::from_uuid(Uuid::new_v4()),
            child,
            owned
        )
        .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), cancel_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(
        sessions
            .tasks()
            .has_active(ConversationId::from_uuid(child))
            .unwrap()
    );
    let failure = TrackingFailure {
        child_conversation_id: child,
        request_id: Uuid::new_v4(),
        turn_id: turn,
        context_snapshot_id: Uuid::new_v4(),
    };
    let parent = Uuid::new_v4();
    let parent_turn = Uuid::new_v4();
    let parent_context = Uuid::new_v4();
    let detail = match failure.tool_error(child, parent, parent_turn, parent_context) {
        research_agent_core::runner::RunError::ToolOutcomeUnknownDiagnostic(value) => {
            serde_json::to_value(value).unwrap()
        }
        other => panic!("expected recovery diagnostic, got {other:?}"),
    };
    assert_eq!(detail["code"], "CHILD_TRACKING_UNCONFIRMED");
    assert_eq!(detail["conversationId"], parent.to_string());
    assert_eq!(detail["turnId"], parent_turn.to_string());
    assert_eq!(detail["contextSnapshotId"], parent_context.to_string());
    assert_eq!(detail["childConversationId"], child.to_string());
    assert_eq!(detail["childTurnId"], turn.to_string());
    assert_eq!(
        detail["childContextSnapshotId"],
        failure.context_snapshot_id.to_string()
    );
    assert_eq!(detail["requestId"], failure.request_id.to_string());
    assert!(matches!(
        failure.tool_error(Uuid::new_v4(), parent, parent_turn, parent_context),
        research_agent_core::runner::RunError::ToolOutcomeUnknown
    ));
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), receipt.wait())
        .await
        .unwrap();
}

#[tokio::test]
async fn ordinary_root_without_call_caps_still_delegates_a_finite_allowance() {
    let root = tempfile::tempdir().unwrap();
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let profile = super::super::execution_profile::execution_profile(
        &ModelIdentity {
            provider_id: "openai".into(),
            model_id: "gpt-5.6-terra".into(),
            revision: None,
        },
        super::super::execution_profile::AgentExecutionMode::ChatGptDefault,
    )
    .unwrap();
    let parent = Uuid::new_v4();
    let snapshot = Uuid::new_v4();
    for (limits, expected) in [
        (profile.limits, (16, 64)),
        (profile.limits.with_call_budget(2, 3), (2, 3)),
    ] {
        let bounds = resolve_parent_bounds(&store, parent, snapshot, limits)
            .await
            .unwrap();
        assert_eq!((bounds.model_calls, bounds.tool_calls), expected);
        assert_eq!(bounds.depth, 0);
    }
}

#[test]
fn bounded_specs_preserve_frozen_scope_and_narrow_budgets() {
    let parent = Uuid::from_u128(1);
    let child = Uuid::from_u128(2);
    let parent_context = Uuid::from_u128(3);
    let child_context = Uuid::from_u128(4);
    let (parent_spec, child_spec) = bounded_specs(SpecInputs {
        parent_conversation_id: parent,
        child_conversation_id: child,
        parent_context_snapshot_id: parent_context,
        child_context_snapshot_id: child_context,
        parent_depth: 0,
        parent_model_calls: 4,
        parent_tool_calls: 6,
        child_model_calls: 9,
        child_tool_calls: 2,
        parent_approved_servers: HashSet::from(["literature".to_owned()]),
        child_approved_servers: HashSet::new(),
    });
    assert_eq!(parent_spec.child_context_snapshot_id, parent_context);
    assert_eq!(child_spec.parent_conversation_id, parent);
    assert_eq!(child_spec.child_conversation_id, child);
    assert_eq!(child_spec.parent_context_snapshot_id, parent_context);
    assert_eq!(child_spec.child_context_snapshot_id, child_context);
    assert_eq!(child_spec.model_calls, 4);
    assert_eq!(
        parent_spec.approved_servers,
        HashSet::from(["literature".to_owned()])
    );
    assert!(child_spec.approved_servers.is_empty());
    assert_eq!(child_spec.tool_calls, 2);
    research_agent_core::subtasks::validate_child(&parent_spec, &child_spec).unwrap();
}

#[tokio::test]
async fn prepare_resume_requires_matching_parent_binding_and_frozen_submission() {
    let root = tempfile::tempdir().unwrap();
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute(
        "insert into note_pages(id,title,created_at,updated_at) values (?1,'Fixture','now','now')",
        ["note"],
    )
    .unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let catalog = SqliteConversationCatalog::from_app_store(&store);
    let parent = Uuid::new_v4();
    let child = Uuid::new_v4();
    let binding = ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "note".to_owned(),
    };
    catalog
        .create_bound_legacy_for_tests(ConversationId::from_uuid(parent), binding.clone())
        .await
        .unwrap();
    catalog
        .create_bound_legacy_for_tests(ConversationId::from_uuid(child), binding.clone())
        .await
        .unwrap();

    let request_id = Uuid::new_v4();
    let submission = super::super::repository::SqliteSubmissionStore::for_app(&store)
        .put(
            child,
            request_id,
            SubmissionInput {
                remote_compaction_v2: None,
                user_input: vec![],
                skill_instructions: vec![],
                text: "resume fixture".to_owned(),
                model: ModelIdentity {
                    provider_id: "fixture".to_owned(),
                    model_id: "model".to_owned(),
                    revision: None,
                },
                reasoning_effort: None,
                service_tier: None,
                context: Vec::new(),
                instructions: Vec::new(),
                instruction_configs: Vec::new(),
                skills: Vec::new(),
            },
        )
        .await
        .unwrap();
    let spec = SubtaskSpec {
        parent_conversation_id: parent,
        child_conversation_id: child,
        parent_context_snapshot_id: Uuid::new_v4(),
        child_context_snapshot_id: submission.context_snapshot_id,
        depth: 1,
        model_calls: 1,
        tool_calls: 1,
        approved_servers: HashSet::new(),
    };
    persist_running(&store, &spec).await.unwrap();
    {
        let mut db = store.connection().unwrap();
        super::super::repository::subtasks::recover_on_startup(&mut db).unwrap();
    }

    let resumed = prepare_resume(&store, &catalog, parent, child, binding.clone())
        .await
        .unwrap();
    assert_eq!(
        resumed.0.child_context_snapshot_id,
        submission.context_snapshot_id
    );
    assert_eq!(resumed.1.request_id, request_id);
    assert!(
        prepare_resume(&store, &catalog, Uuid::new_v4(), child, binding.clone())
            .await
            .is_err()
    );
    assert!(
        prepare_resume(
            &store,
            &catalog,
            parent,
            child,
            ConversationBinding {
                kind: BindingKind::ReadingNote,
                resource_id: "note".to_owned()
            },
        )
        .await
        .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn recovered_child_scope_reuses_runtime_without_global_expansion() {
    use crate::agent::{
        mcp_config::{ConfiguredServer, Transport},
        mcp_runtime::McpRuntime,
    };
    use rusqlite::Connection;
    use std::{collections::BTreeMap, ffi::OsString, path::PathBuf, time::Duration};

    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let store = AppStore::for_test(db, root_path.join("objects"));
    let script = r#"while IFS= read line; do case "$line" in *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;; *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"search","inputSchema":{"type":"object"}}]}}' ;; esac; done"#;
    let config_root = store.agent_configuration_directory().join("agent");
    std::fs::create_dir_all(&config_root).unwrap();
    std::fs::write(
        config_root.join("mcp.toml"),
        format!(
            "schema_version=1\n[servers.literature]\nenvironment_id='test'\nrevision='r1'\nprogram='/bin/sh'\ndirectory='{}'\n",
            root_path.display()
        ),
    )
    .unwrap();
    {
        let db = store.connection().unwrap();
        crate::agent::repository::mcp_approvals::upsert(
            &db,
            &crate::agent::repository::mcp_approvals::Approval {
                server_id: "literature".into(),
                environment_id: "test".into(),
                revision: "r1".into(),
            },
        )
        .unwrap();
    }
    assert!(
        crate::agent::mcp_config::read_from_root(store.agent_configuration_directory())
            .unwrap()
            .is_some()
    );
    let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());
    let runtime = McpRuntime::build(
        vec![ConfiguredServer {
            server_id: "literature".into(),
            environment_id: "test".into(),
            revision: "r1".into(),
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![OsString::from("-c"), OsString::from(script)],
                directory: root_path.clone(),
            },
            environment: BTreeMap::new(),
            credential_handle: None,
            enabled: true,
        }],
        &HashSet::from(["literature".to_owned()]),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    sessions.install_mcp_runtime(Arc::new(runtime)).await;

    let parent = Uuid::new_v4();
    let child = Uuid::new_v4();
    let context = Uuid::new_v4();
    let spec = SubtaskSpec {
        parent_conversation_id: parent,
        child_conversation_id: child,
        parent_context_snapshot_id: context,
        child_context_snapshot_id: context,
        depth: 1,
        model_calls: 1,
        tool_calls: 1,
        approved_servers: HashSet::from(["literature".to_owned()]),
    };
    persist_running(&store, &spec).await.unwrap();
    let scoped = mcp_runtime_for_conversation(&store, &sessions, child)
        .await
        .unwrap();
    assert_eq!(scoped.definitions().len(), 1);
    assert_eq!(
        scoped.approved_servers(),
        HashSet::from(["literature".to_owned()])
    );

    std::fs::write(
        config_root.join("mcp.toml"),
        std::fs::read_to_string(config_root.join("mcp.toml"))
            .unwrap()
            .replace("revision='r1'", "revision='r2'"),
    )
    .unwrap();
    assert!(
        mcp_runtime_for_conversation(&store, &sessions, child)
            .await
            .is_none()
    );
    assert!(sessions.mcp_runtime().await.is_none());
}
