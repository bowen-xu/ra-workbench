use super::*;
use crate::agent::repository::{append_batch, create_conversation};
use research_agent_core::conversation::ConversationForkRequest;
use research_agent_core::protocol::{AgentEvent, EventPayload as P, ModelIdentity, TurnOutcome};

fn id(n: u128) -> ConversationId {
    ConversationId::from_uuid(Uuid::from_u128(n))
}
fn request(n: u128) -> Request {
    Request {
        request_id: Uuid::from_u128(n + 100),
        conversation_id: id(n),
        binding: Some(ConversationBinding {
            kind: BindingKind::Manuscript,
            resource_id: "m".into(),
        }),
        expected_title: "Delete me".into(),
        expected_durable_count: 0,
        force: false,
    }
}
fn seed(db: &Connection, n: u128) {
    db.execute(
        "insert into agent_conversations(id,title,archived) values(?1,'Delete me',1)",
        [id(n).as_uuid().to_string()],
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript','m')", [id(n).as_uuid().to_string()]).unwrap();
}
fn setup() -> (Arc<Mutex<Connection>>, SqliteConversationCatalog) {
    initialize(Connection::open_in_memory().unwrap())
}
#[tokio::test]
async fn global_deletion_checks_archival_and_metadata_and_retries_without_a_binding() {
    let (db, catalog) = setup();
    db.lock()
        .unwrap()
        .execute("delete from agent_conversation_bindings", [])
        .unwrap();
    let mut global = request(1);
    global.binding = None;
    catalog
        .archive_legacy_for_tests(id(1), false)
        .await
        .unwrap();
    assert_eq!(
        catalog
            .delete_legacy_for_tests(global.clone())
            .await
            .unwrap_err(),
        Error::NotArchived
    );
    catalog.archive_legacy_for_tests(id(1), true).await.unwrap();
    let mut stale = global.clone();
    stale.expected_durable_count = 1;
    assert_eq!(
        catalog.delete_legacy_for_tests(stale).await.unwrap_err(),
        Error::Catalog(CatalogError::Conflict)
    );
    let deleted = catalog
        .delete_legacy_for_tests(global.clone())
        .await
        .unwrap();
    assert_eq!(
        catalog.delete_legacy_for_tests(global).await.unwrap(),
        deleted
    );
    assert_eq!(count(&db.lock().unwrap(), "agent_conversations"), 0);
    assert_eq!(count(&db.lock().unwrap(), "manuscripts"), 1);
}
fn initialize(mut db: Connection) -> (Arc<Mutex<Connection>>, SqliteConversationCatalog) {
    crate::storage_migration_tests::migrate_through(&mut db, 119);
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now');
        insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','Manuscript','now','now')").unwrap();
    seed(&db, 1);
    let db = Arc::new(Mutex::new(db));
    (db.clone(), SqliteConversationCatalog::new(db))
}
fn count(db: &Connection, table: &str) -> i64 {
    // Only static test-owned table names.
    db.query_row(&format!("select count(*) from {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
fn history(db: &mut Connection, finished: bool) {
    db.execute("update agent_conversations set archived=0", [])
        .unwrap();
    let mut payloads = vec![P::TurnStarted {
        task_kind: research_agent_core::protocol::TaskKind::Regular,
        turn_id: Uuid::from_u128(10),
        context_snapshot_id: Uuid::from_u128(11),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "fixture".into(),
            revision: None,
        },
    }];
    if finished {
        payloads.push(P::TurnFinished {
            turn_id: Uuid::from_u128(10),
            outcome: TurnOutcome::Completed,
        });
    }
    let events: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(id(1), index as u64 + 1, 0, Uuid::from_u128(9), payload).unwrap()
        })
        .collect();
    append_batch(db, id(1), Uuid::new_v4(), 0, &events).unwrap();
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
}
fn proposal(db: &Connection, owner: u128, change: &str) {
    let key = id(owner).as_uuid().to_string();
    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')",
        params![key,format!("request-{owner}"),format!("turn-{owner}"),format!("context-{owner}")]).unwrap();
    db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version)
        values(?1,?2,?3,?4,?5,'m',?6,?7,?7,1)", params![change,key,format!("request-{owner}"),format!("turn-{owner}"),format!("context-{owner}"),format!("lease-{owner}"),"a".repeat(64)]).unwrap();
}

#[tokio::test]
async fn deletion_is_atomic_idempotent_and_permanently_fences_stale_creators() {
    let (db, catalog) = setup();
    let (a, b) = tokio::join!(
        catalog.delete_legacy_for_tests(request(1)),
        catalog.delete_legacy_for_tests(request(1))
    );
    assert_eq!(a.unwrap(), b.unwrap());
    let reopened = SqliteConversationCatalog::new(db.clone());
    assert_eq!(
        reopened
            .delete_legacy_for_tests(request(1))
            .await
            .unwrap()
            .conversation_id,
        id(1)
    );
    let mut wrong = request(1);
    wrong.expected_title = "Other title".into();
    assert_eq!(
        reopened.delete_legacy_for_tests(wrong).await.unwrap_err(),
        Error::Catalog(CatalogError::Conflict)
    );
    assert!(
        catalog
            .create_bound_legacy_for_tests(id(1), request(1).binding.unwrap())
            .await
            .is_err()
    );
    let mut db = db.lock().unwrap();
    assert!(create_conversation(&mut db, id(1), "Resurrected").is_err());
    assert_eq!(count(&db, "agent_conversations"), 0);
    assert_eq!(count(&db, "agent_conversation_deletions"), 1);
    assert_eq!(count(&db, "manuscripts"), 1);
    assert_eq!(count(&db, "research_projects"), 1);
}
#[tokio::test]
async fn validates_archive_confirmation_binding_and_active_history_before_deletion() {
    let (db, catalog) = setup();
    catalog
        .archive_legacy_for_tests(id(1), false)
        .await
        .unwrap();
    assert_eq!(
        catalog
            .delete_legacy_for_tests(request(1))
            .await
            .unwrap_err(),
        Error::NotArchived
    );
    catalog.archive_legacy_for_tests(id(1), true).await.unwrap();
    let mut wrong = request(1);
    wrong.expected_title = "Stale".into();
    assert_eq!(
        catalog.delete_legacy_for_tests(wrong).await.unwrap_err(),
        Error::Catalog(CatalogError::Conflict)
    );
    let mut wrong = request(1);
    wrong.binding.as_mut().unwrap().resource_id = "other".into();
    assert_eq!(
        catalog.delete_legacy_for_tests(wrong).await.unwrap_err(),
        Error::Catalog(CatalogError::NotFound)
    );
    history(&mut db.lock().unwrap(), false);
    let mut active = request(1);
    active.expected_durable_count = 1;
    assert_eq!(
        catalog.delete_legacy_for_tests(active).await.unwrap_err(),
        Error::Catalog(CatalogError::Busy)
    );
    assert_eq!(
        count(&db.lock().unwrap(), "agent_conversation_deletions"),
        0
    );
}
#[tokio::test]
async fn child_deletion_releases_parent_provenance_but_never_deletes_the_manuscript() {
    let (db, catalog) = setup();
    history(&mut db.lock().unwrap(), true);
    let child = catalog
        .fork_legacy_for_tests(ConversationForkRequest {
            child_id: id(2),
            parent_id: id(1),
            through_turn_id: Uuid::from_u128(10),
            binding: request(1).binding.unwrap(),
        })
        .await
        .unwrap();
    let mut parent = request(1);
    parent.expected_durable_count = 2;
    assert_eq!(
        catalog
            .delete_legacy_for_tests(parent.clone())
            .await
            .unwrap_err(),
        Error::Retained(Retention::BranchHistory)
    );
    catalog.archive_legacy_for_tests(id(2), true).await.unwrap();
    let mut child_request = request(2);
    child_request.expected_title = child.conversation.title;
    child_request.expected_durable_count = 2;
    catalog
        .delete_legacy_for_tests(child_request)
        .await
        .unwrap();
    catalog.delete_legacy_for_tests(parent).await.unwrap();
    let db = db.lock().unwrap();
    assert_eq!(count(&db, "agent_items"), 0);
    assert_eq!(count(&db, "agent_journal_batches"), 0);
    assert_eq!(count(&db, "agent_fork_turn_sources"), 0);
    assert_eq!(count(&db, "manuscripts"), 1);
}
#[tokio::test]
async fn retained_applications_and_execution_workspaces_are_not_cascaded_away() {
    let (db, catalog) = setup();
    {
        let db = db.lock().unwrap();
        proposal(&db, 1, "change");
        db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('application','change','apply')",[]).unwrap();
    }
    catalog.delete_legacy_for_tests(request(1)).await.unwrap();
    assert_eq!(count(&db.lock().unwrap(), "agent_proposal_applications"), 1);
    assert_eq!(count(&db.lock().unwrap(), "agent_change_sets"), 1);
    assert_eq!(count(&db.lock().unwrap(), "agent_submissions"), 0);
    {
        let db = db.lock().unwrap();
        seed(&db, 2);
        proposal(&db, 2, "second");
        db.execute("insert into agent_execution_workspaces(conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json,state)
            values(?1,'request-2','turn-2','context-2','m','lease-2',?2,'{}','revoked')",params![id(2).as_uuid().to_string(),"a".repeat(64)]).unwrap();
    }
    assert_eq!(
        catalog
            .delete_legacy_for_tests(request(2))
            .await
            .unwrap_err(),
        Error::Retained(Retention::ExecutionWorkspace)
    );
    let mut force = request(2);
    force.force = true;
    catalog.delete_legacy_for_tests(force).await.unwrap();
    assert_eq!(count(&db.lock().unwrap(), "agent_execution_workspaces"), 0);
    assert_eq!(
        count(&db.lock().unwrap(), "agent_conversation_deletions"),
        2
    );
}
#[tokio::test]
async fn removing_last_unapplied_proposal_reference_schedules_existing_gc_grace() {
    let (db, catalog) = setup();
    {
        let db = db.lock().unwrap();
        seed(&db, 2);
        proposal(&db, 1, "first");
        proposal(&db, 2, "second");
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'fixture','text/plain',1,'now')",["b".repeat(64)]).unwrap();
        for change in ["first", "second"] {
            db.execute("insert into agent_change_set_files(change_set_id,relative_path,after_sha256,after_executable) values(?1,'new.tex',?2,0)",params![change,"b".repeat(64)]).unwrap();
        }
    }
    catalog.delete_legacy_for_tests(request(1)).await.unwrap();
    assert_eq!(
        count(&db.lock().unwrap(), "content_object_gc_candidates"),
        0
    );
    catalog.delete_legacy_for_tests(request(2)).await.unwrap();
    let db = db.lock().unwrap();
    assert_eq!(count(&db, "content_objects"), 1);
    assert_eq!(count(&db, "content_object_gc_candidates"), 1);
    let grace:f64=db.query_row("select julianday(not_before)-julianday(unreferenced_since) from content_object_gc_candidates",[],|r|r.get(0)).unwrap();
    assert!(grace >= 30.0);
}
#[tokio::test]
async fn failed_delete_rolls_back_its_receipt_and_cascade() {
    let (db, catalog) = setup();
    db.lock().unwrap().execute_batch("create trigger fail_delete before delete on agent_conversations begin select raise(abort,'fixture failure'); end;").unwrap();
    assert!(catalog.delete_legacy_for_tests(request(1)).await.is_err());
    {
        let db = db.lock().unwrap();
        assert_eq!(count(&db, "agent_conversations"), 1);
        assert_eq!(count(&db, "agent_conversation_deletions"), 0);
        db.execute_batch("drop trigger fail_delete").unwrap();
    }
    catalog.delete_legacy_for_tests(request(1)).await.unwrap();
}

#[tokio::test]
async fn receipt_survives_database_reopen_and_cannot_be_removed_by_cleanup() {
    let file = tempfile::NamedTempFile::new().unwrap();
    {
        let (_, catalog) = initialize(Connection::open(file.path()).unwrap());
        catalog.delete_legacy_for_tests(request(1)).await.unwrap();
    }
    let connection = Connection::open(file.path()).unwrap();
    connection.execute_batch("pragma foreign_keys=on").unwrap();
    assert_eq!(count(&connection, "agent_conversations"), 0);
    assert_eq!(count(&connection, "agent_conversation_deletions"), 1);
    assert!(
        connection
            .execute("delete from agent_conversation_deletions", [])
            .is_err()
    );
    assert!(
        connection
            .execute(
                "update agent_conversation_deletions set deleted_at='changed'",
                []
            )
            .is_err()
    );
    let catalog = SqliteConversationCatalog::new(Arc::new(Mutex::new(connection)));
    assert_eq!(
        catalog
            .delete_legacy_for_tests(request(1))
            .await
            .unwrap()
            .conversation_id,
        id(1)
    );
    assert!(
        catalog
            .create_bound_legacy_for_tests(id(1), request(1).binding.unwrap())
            .await
            .is_err()
    );
}

#[test]
fn confirmed_cleanup_with_refused_sql_has_its_own_partial_operation_code() {
    let operation = Uuid::new_v4();
    let error = failure(DeletionFailure::CleanupComplete(Box::new(
        crate::data_interface::SqlImmediateDiagnostic {
            operation_id: operation,
            epoch: Some(Uuid::new_v4()),
            sql_fence: Some("7".into()),
            phase: "delete-publication",
            sqlite_code: None,
            sqlite_extended_code: None,
            safe_cause: "delete_not_archived_after_workspace_cleanup",
        },
    )));
    let wire =
        serde_json::to_value(crate::agent::deletion_commands::DeleteError::from(error)).unwrap();
    assert_eq!(wire["reason"]["code"], "DATA_OPERATION_PARTIAL");
    assert_eq!(
        wire["reason"]["reason"]["operationId"],
        operation.to_string()
    );
}
#[test]
fn uncertain_file_effects_are_not_reported_as_a_sql_commit() {
    let operation = Uuid::new_v4();
    let error = workspace_failure(
        operation,
        research_agent_core::conversation::CatalogDataFailureKind::ExternalUnknown,
        "delete-workspace-discard",
        "workspace_discard_unconfirmed",
    );
    let wire =
        serde_json::to_value(crate::agent::deletion_commands::DeleteError::from(error)).unwrap();
    assert_eq!(wire["reason"]["code"], "DATA_OPERATION_OUTCOME_UNKNOWN");
    assert_eq!(
        wire["reason"]["reason"]["operationId"],
        operation.to_string()
    );
    assert_eq!(
        wire["reason"]["reason"]["phase"],
        "delete-workspace-discard"
    );
}
