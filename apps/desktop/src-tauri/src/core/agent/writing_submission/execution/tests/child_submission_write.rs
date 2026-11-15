use super::child_cleanup::ChildTransport;
use super::*;
use crate::{ai::JsonCredentialStore, data_interface::Database};
use research_agent_core::protocol::ItemKind;

#[tokio::test]
async fn child_submission_commit_unknown_never_archives_or_resamples_and_reopens() {
    let mut f = super::super::super::tests::fixture_at(true, tempfile::tempdir().unwrap());
    let path = f.root.path().join("child-submission.sqlite3");
    let mut disk = rusqlite::Connection::open(&path).unwrap();
    {
        let legacy = f.store.connection().unwrap();
        rusqlite::backup::Backup::new(&legacy, &mut disk)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
    }
    crate::storage::migrate(&mut disk).unwrap();
    let log = f.root.path().join("child-submission-log");
    let db = Database::open(disk, &log).unwrap();
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
    f.store.connection().unwrap().execute_batch("create table fixture_submission_parent(id integer primary key);
      create table fixture_submission_deferred(id integer references fixture_submission_parent(id) deferrable initially deferred);
      create trigger fixture_submission_commit_unknown after insert on data_sql_operation_receipts when new.kind='agent.child.submission' begin insert into fixture_submission_deferred(id) values(9); end;").unwrap();
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
    let reopened_db = Database::open(rusqlite::Connection::open(&path).unwrap(), &log).unwrap();
    let reopened_store =
        AppStore::for_test_database(reopened_db.clone(), f.root.path().join("data/objects"));
    let sessions = AgentSessionService::from_app_store(&reopened_store).unwrap();
    let history = sessions
        .open(ConversationId::from_uuid(f.conversation))
        .await
        .unwrap();
    assert!(history.has_unknown_tool_outcome());
    let turn = history.turn(record.turn_id).unwrap();
    let call_id = *turn.child_creation_intents.keys().next().unwrap();
    assert!(!turn.items.values().any(|item| item.completed
        && matches!(item.kind, ItemKind::ToolResult {call_id: id, ..} if id == call_id)));
    let child = *turn.child_creation_intents[&call_id].id.as_uuid();
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
    assert!(wire.get("cleanup").is_none());
    let sql = reopened_db.lock().unwrap();
    let facts: (bool,u64,u64) = sql.query_row("select (select archived from agent_conversations where id=?1), (select count(*) from agent_child_cleanup_intents), (select count(*) from data_sql_operation_receipts where kind='agent.conversation.archive')", [child.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(facts, (false, 0, 0));
}
