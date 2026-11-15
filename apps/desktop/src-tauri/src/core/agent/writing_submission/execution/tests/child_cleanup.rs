use super::*;
use research_agent_core::conversation::{ArchiveConversation, ArchiveStatus, ConversationCatalog};
use research_agent_core::protocol::{FailureCategory, ItemKind};

pub(super) struct ChildTransport(pub(super) AtomicUsize);

impl ModelTransport for ChildTransport {
    fn capabilities(&self) -> ModelCapabilities {
        Transport(AtomicUsize::new(0)).capabilities()
    }

    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        let index = self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(
                index, 0,
                "cleanup failure must not trigger another model request"
            );
            Ok(Box::new(Stream(
                vec![
                    ModelStreamEvent::Started {
                        response_id: "child-cleanup-fixture".into(),
                        actual_model: request.model,
                    },
                    ModelStreamEvent::ToolCallStarted {
                        call_id: "child-cleanup-call".into(),
                        name: "research_spawn_subtask".into(),
                    },
                    ModelStreamEvent::ToolArgumentsDelta {
                        call_id: "child-cleanup-call".into(),
                        delta: r#"{"prompt":"Compare approaches"}"#.into(),
                    },
                    ModelStreamEvent::ToolCallCompleted {
                        call_id: "child-cleanup-call".into(),
                    },
                    ModelStreamEvent::Completed {
                        reason: FinishReason::ToolCalls,
                        end_turn: None,
                    },
                ]
                .into(),
            )) as Box<dyn ModelStream>)
        })
    }
}

#[tokio::test]
async fn child_submission_and_cleanup_failure_stops_the_real_parent_runner() {
    let f = fixture(true);
    let service =
        WritingSubmissionService::from_app_store(&f.store).with_model_resolver(f.models.clone());
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    f.store.connection().unwrap().execute_batch("create trigger fixture_child_submission_failure before insert on agent_submissions begin select raise(abort, 'synthetic submission failure'); end;
      create trigger fixture_child_archive_failure before update of archived on agent_conversations begin select raise(abort, 'synthetic archive failure'); end;").unwrap();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let transport = Arc::new(ChildTransport(AtomicUsize::new(0)));
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
        panic!("expected started parent")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    assert_eq!(transport.0.load(Ordering::SeqCst), 1);
    let projection = sessions
        .read_projection(ConversationId::from_uuid(f.conversation))
        .await
        .unwrap();
    let turn = projection.turn(record.turn_id).unwrap();
    assert!(projection.has_unknown_tool_outcome());
    assert_eq!(turn.outcome, Some(TurnOutcome::Failed));
    let (error, category) = turn.failure.as_ref().unwrap();
    assert_eq!(*category, FailureCategory::Recovery);
    let diagnostic: serde_json::Value =
        serde_json::from_str(error.strip_prefix("tool_outcome_unknown: ").unwrap()).unwrap();
    let intent = turn.child_creation_intents.values().next().unwrap();
    assert_eq!(diagnostic["resultId"], intent.id.as_uuid().to_string());
    let child_request = Uuid::parse_str(diagnostic["requestId"].as_str().unwrap()).unwrap();
    assert_eq!(
        turn.child_submission_requests.values().next(),
        Some(&child_request)
    );
    let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
    phase.extend_from_slice(child_request.as_bytes());
    let operation = Uuid::new_v5(intent.id.as_uuid(), &phase);
    assert_eq!(diagnostic["journalRequestId"], operation.to_string());
    assert_eq!(diagnostic["childArchiveIntentVersion"], 1);
    assert_eq!(diagnostic["childArchiveOperationId"], operation.to_string());
    assert_eq!(diagnostic["childArchiveExpectedWatermark"], 0);
    let catalog = crate::agent::repository::SqliteConversationCatalog::from_app_store(&f.store);
    assert!(matches!(
        catalog
            .archive_status(ArchiveConversation {
                operation_id: operation,
                id: intent.id,
                archived: true,
                expected_watermark: Some(0)
            })
            .await
            .unwrap(),
        ArchiveStatus::NoSqlReceipt { .. }
    ));
    assert!(
        !catalog
            .read(intent.id, Some(intent.binding.clone()))
            .await
            .unwrap()
            .archived
    );
    assert!(
        !turn
            .items
            .values()
            .any(|item| matches!(item.kind, ItemKind::ToolResult { .. }) && item.completed)
    );
    assert_eq!(turn.child_creation_intents.len(), 1);
    let call_id = *turn.child_creation_intents.keys().next().unwrap();
    let status = crate::agent::child_creation::status::execute(
        &sessions,
        &catalog,
        crate::agent::child_creation::status::Request {
            schema_version: 1,
            conversation_id: f.conversation,
            turn_id: record.turn_id,
            call_id,
        },
    )
    .await
    .unwrap();
    let status = serde_json::to_value(status).unwrap();
    assert_eq!(status["creation"]["status"]["type"], "committed");
    assert_eq!(status["cleanup"]["operationId"], operation.to_string());
    assert_eq!(status["cleanup"]["status"]["type"], "noSqlReceipt");
    assert!(projection.has_unknown_tool_outcome());
    assert!(matches!(
        turn.items
            .values()
            .find(|item| matches!(item.kind, ItemKind::ToolCall { .. }))
            .map(|item| item.completed),
        Some(true)
    ));
}

#[tokio::test]
async fn cleanup_commit_unknown_blocks_terminal_diagnostic_after_owner_reopen() {
    use crate::{ai::JsonCredentialStore, data_interface::Database};
    let crash_root =
        std::env::var_os("RESEARCH_CHILD_CLEANUP_CRASH_ROOT").map(std::path::PathBuf::from);
    let fixture_root = if let Some(root) = &crash_root {
        assert_eq!(
            std::fs::read(root.join("marker")).unwrap(),
            b"synthetic-child-cleanup-v1"
        );
        tempfile::tempdir_in(root).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let mut f = super::super::super::tests::fixture_at(true, fixture_root);
    let database_path = f.root.path().join("cleanup.sqlite3");
    let mut disk = rusqlite::Connection::open(&database_path).unwrap();
    {
        let legacy = f.store.connection().unwrap();
        rusqlite::backup::Backup::new(&legacy, &mut disk)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
    }
    crate::storage::migrate(&mut disk).unwrap();
    let log_path = f.root.path().join("cleanup-log");
    let db = Database::open(disk, &log_path).unwrap();
    f.store = AppStore::for_test_database(db.clone(), f.root.path().join("data/objects"));
    f.models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(f.root.path().join("credentials.json")),
    );
    let service =
        WritingSubmissionService::from_app_store(&f.store).with_model_resolver(f.models.clone());
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    f.store.connection().unwrap().execute_batch("create trigger fixture_child_submission_failure before insert on agent_submissions begin select raise(abort, 'synthetic submission failure'); end;
      create table fixture_cleanup_parent(id integer primary key);
      create table fixture_cleanup_deferred(id integer references fixture_cleanup_parent(id) deferrable initially deferred);
      create trigger fixture_cleanup_commit_unknown after insert on data_sql_operation_receipts when new.kind='agent.conversation.archive' begin insert into fixture_cleanup_deferred(id) values(9); end;").unwrap();
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(f.store.shared_agent_connection(), "fixture".into()).unwrap(),
    )));
    let transport = Arc::new(ChildTransport(AtomicUsize::new(0)));
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
        panic!("expected started parent")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    assert_eq!(transport.0.load(Ordering::SeqCst), 1);
    assert!(
        f.store.connection().is_err(),
        "commit unknown blocks the original owner"
    );
    let conversation = f.conversation;
    if let Some(root) = crash_root {
        let identity = serde_json::json!({"fixture":f.root.path(),"conversation":conversation,"turn":record.turn_id});
        std::fs::write(
            root.join("identity.json"),
            serde_json::to_vec(&identity).unwrap(),
        )
        .unwrap();
        // Simulate abrupt termination: the blocked SQLite transaction and all owners are dropped by the OS.
        std::process::exit(0);
    }
    drop(owned);
    drop(transport);
    drop(service);
    drop(sessions);
    drop(f.models);
    drop(f.store);
    drop(db);
    inspect_reopened(f.root.path(), conversation, record.turn_id).await;
}

#[tokio::test]
async fn preparation_commit_unknown_stops_before_archive_and_reopens_with_barrier() {
    use crate::{ai::JsonCredentialStore, data_interface::Database};
    let mut f = super::super::super::tests::fixture_at(true, tempfile::tempdir().unwrap());
    let database_path = f.root.path().join("prepare-unknown.sqlite3");
    let mut disk = rusqlite::Connection::open(&database_path).unwrap();
    {
        let legacy = f.store.connection().unwrap();
        rusqlite::backup::Backup::new(&legacy, &mut disk)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
    }
    crate::storage::migrate(&mut disk).unwrap();
    let log_path = f.root.path().join("prepare-unknown-log");
    let db = Database::open(disk, &log_path).unwrap();
    f.store = AppStore::for_test_database(db.clone(), f.root.path().join("data/objects"));
    f.models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(f.root.path().join("credentials.json")),
    );
    let service =
        WritingSubmissionService::from_app_store(&f.store).with_model_resolver(f.models.clone());
    let record = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap()
        .record;
    f.store.connection().unwrap().execute_batch("create trigger fixture_child_submission_failure before insert on agent_submissions begin select raise(abort, 'synthetic submission failure'); end;
      create table fixture_prepare_parent(id integer primary key);
      create table fixture_prepare_deferred(id integer references fixture_prepare_parent(id) deferrable initially deferred);
      create trigger fixture_prepare_commit_unknown after insert on data_sql_operation_receipts when new.kind='agent.child.cleanup.prepare' begin insert into fixture_prepare_deferred(id) values(9); end;").unwrap();
    let sessions = Arc::new(AgentSessionService::from_app_store(&f.store).unwrap());
    let transport = Arc::new(ChildTransport(AtomicUsize::new(0)));
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
        panic!("expected started parent")
    };
    assert_eq!(owned.task.wait().await, TaskCompletion::RecoveryRequired);
    assert_eq!(transport.0.load(Ordering::SeqCst), 1);
    assert!(f.store.connection().is_err());
    drop(owned);
    drop(transport);
    drop(service);
    drop(sessions);
    drop(f.models);
    drop(f.store);
    drop(db);
    let reopened_db = Database::open(
        rusqlite::Connection::open(&database_path).unwrap(),
        &log_path,
    )
    .unwrap();
    let reopened_store =
        AppStore::for_test_database(reopened_db.clone(), f.root.path().join("data/objects"));
    let sessions = AgentSessionService::from_app_store(&reopened_store).unwrap();
    let projection = sessions
        .open(ConversationId::from_uuid(f.conversation))
        .await
        .unwrap();
    assert!(projection.has_unknown_tool_outcome());
    let turn = projection.turn(record.turn_id).unwrap();
    let call_id = *turn.child_creation_intents.keys().next().unwrap();
    let child = *turn.child_creation_intents[&call_id].id.as_uuid();
    assert_eq!(
        turn.child_submission_requests.get(&call_id),
        Some(&research_agent_core::conversation::child_submission_request_id(child))
    );
    assert!(
        turn.failure
            .as_ref()
            .is_none_or(|(message, _)| !message.contains("childArchiveIntentVersion"))
    );
    let catalog =
        crate::agent::repository::SqliteConversationCatalog::from_app_store(&reopened_store);
    let status = crate::agent::child_creation::status::execute(
        &sessions,
        &catalog,
        crate::agent::child_creation::status::Request {
            schema_version: 1,
            conversation_id: f.conversation,
            turn_id: record.turn_id,
            call_id,
        },
    )
    .await
    .unwrap();
    let wire = serde_json::to_value(status).unwrap();
    assert_eq!(wire["creation"]["status"]["type"], "committed");
    assert_eq!(
        wire["submission"]["requestId"],
        research_agent_core::conversation::child_submission_request_id(child).to_string()
    );
    assert_eq!(wire["submission"]["record"], serde_json::Value::Null);
    assert!(
        wire.get("cleanup").is_none(),
        "rolled-back preparation cannot supply an archive identity"
    );
    let sql = reopened_db.lock().unwrap();
    let (preparations, archives): (u64, u64) = sql.query_row(
        "select (select count(*) from agent_child_cleanup_intents), (select count(*) from data_sql_operation_receipts where kind='agent.conversation.archive')",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!((preparations, archives), (0, 0));
    assert!(
        !sql.query_row(
            "select archived from agent_conversations where id=?1",
            [child.to_string()],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    );
    sql.execute_batch("drop trigger fixture_prepare_commit_unknown;")
        .unwrap();
    drop(sql);
    let wrong_request = Uuid::new_v4();
    let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
    phase.extend_from_slice(wrong_request.as_bytes());
    reopened_db
        .prepare_child_cleanup(&crate::data_interface::ChildCleanupIntent {
            request_id: wrong_request,
            archive: ArchiveConversation {
                operation_id: Uuid::new_v5(&child, &phase),
                id: ConversationId::from_uuid(child),
                archived: true,
                expected_watermark: Some(0),
            },
        })
        .unwrap();
    let error = crate::agent::child_creation::status::execute(
        &sessions,
        &catalog,
        crate::agent::child_creation::status::Request {
            schema_version: 1,
            conversation_id: f.conversation,
            turn_id: record.turn_id,
            call_id,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.kind,
        crate::agent::child_creation::status::ErrorKind::IdentityMismatch
    ));
}

async fn inspect_reopened(root: &std::path::Path, conversation: Uuid, turn_id: Uuid) {
    use crate::data_interface::Database;
    let reopened_db = Database::open(
        rusqlite::Connection::open(root.join("cleanup.sqlite3")).unwrap(),
        &root.join("cleanup-log"),
    )
    .unwrap();
    let reopened_store =
        AppStore::for_test_database(reopened_db.clone(), root.join("data/objects"));
    let reopened = Arc::new(AgentSessionService::from_app_store(&reopened_store).unwrap());
    let projection = reopened
        .open(ConversationId::from_uuid(conversation))
        .await
        .unwrap();
    let turn = projection.turn(turn_id).unwrap();
    assert!(projection.has_unknown_tool_outcome());
    assert_eq!(turn.child_creation_intents.len(), 1);
    assert!(
        turn.failure
            .as_ref()
            .is_none_or(|(message, _)| !message.contains("childArchiveIntentVersion")),
        "blocked owner must not fabricate a saved cleanup intent"
    );
    let call_id = *turn.child_creation_intents.keys().next().unwrap();
    let catalog = crate::agent::repository::SqliteConversationCatalog::new(reopened_db.clone());
    let status = crate::agent::child_creation::status::execute(
        &reopened,
        &catalog,
        crate::agent::child_creation::status::Request {
            schema_version: 1,
            conversation_id: conversation,
            turn_id,
            call_id,
        },
    )
    .await
    .unwrap();
    let wire = serde_json::to_value(status).unwrap();
    assert_eq!(wire["creation"]["status"]["type"], "committed");
    assert_eq!(wire["cleanup"]["status"]["type"], "noSqlReceipt");
    let saved: (String, String, String, u64, String) = reopened_db.lock().unwrap().query_row(
        "select child_id,request_id,archive_operation_id,expected_watermark,epoch from agent_child_cleanup_intents", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    ).unwrap();
    assert_eq!(wire["cleanup"]["conversationId"], saved.0);
    assert_eq!(wire["cleanup"]["operationId"], saved.2);
    assert_eq!(wire["cleanup"]["status"]["epoch"], saved.4);
    assert_eq!(saved.3, 0);
}

#[tokio::test]
async fn cleanup_commit_unknown_survives_process_exit_with_original_intent() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("marker"), b"synthetic-child-cleanup-v1").unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "agent::writing_submission::execution::tests::child_cleanup::cleanup_commit_unknown_blocks_terminal_diagnostic_after_owner_reopen", "--nocapture", "--test-threads=1"])
        .env("RESEARCH_CHILD_CLEANUP_CRASH_ROOT", root.path()).output().unwrap();
    assert!(
        child.status.success(),
        "synthetic subprocess failed: {}",
        String::from_utf8_lossy(&child.stderr)
    );
    let identity: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("identity.json")).unwrap()).unwrap();
    let fixture = std::path::PathBuf::from(identity["fixture"].as_str().unwrap());
    assert!(fixture.starts_with(root.path()));
    let conversation = Uuid::parse_str(identity["conversation"].as_str().unwrap()).unwrap();
    let turn = Uuid::parse_str(identity["turn"].as_str().unwrap()).unwrap();
    inspect_reopened(&fixture, conversation, turn).await;
}
