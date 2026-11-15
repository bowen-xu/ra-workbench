use super::*;
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ModelIdentity, TurnOutcome},
};

fn setup(db: &mut Connection, version: i64) -> Acknowledge {
    crate::storage_migration_tests::migrate_through(db, version);
    let scope = Scope {
        manuscript_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        request_id: Uuid::new_v4(),
        context_snapshot_id: Uuid::new_v4(),
    };
    let turn_id = Uuid::new_v4();
    db.execute_batch(
        "insert into research_projects(id,title,summary,created_at) values('p','p','','now')",
    )
    .unwrap();
    db.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values(?1,'p','paper','now','now')", [scope.manuscript_id.to_string()]).unwrap();
    super::super::repository::create_conversation(
        db,
        ConversationId::from_uuid(scope.conversation_id),
        "test",
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)", params![scope.conversation_id.to_string(),scope.manuscript_id.to_string()]).unwrap();
    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')",params![scope.conversation_id.to_string(),scope.request_id.to_string(),turn_id.to_string(),scope.context_snapshot_id.to_string()]).unwrap();
    let result = Acknowledge { scope, turn_id };
    workspace(db, &result);
    result
}
fn workspace(db: &Connection, request: &Acknowledge) {
    let s = &request.scope;
    db.execute("insert into agent_execution_workspaces(conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json) values(?1,?2,?3,?4,?5,?6,?7,'{}')", params![s.conversation_id.to_string(),s.request_id.to_string(),request.turn_id.to_string(),s.context_snapshot_id.to_string(),s.manuscript_id.to_string(),Uuid::new_v4().to_string(),"0".repeat(64)]).unwrap();
}
fn append(db: &Connection, request: &Acknowledge, seq: u64, payload: EventPayload) {
    let event = AgentEvent::new(
        ConversationId::from_uuid(request.scope.conversation_id),
        seq,
        0,
        request.scope.request_id,
        payload,
    )
    .unwrap();
    db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)",params![request.scope.conversation_id.to_string(),seq-1,serde_json::to_string(&event).unwrap()]).unwrap();
}
fn start(db: &Connection, request: &Acknowledge) {
    append(
        db,
        request,
        1,
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: request.turn_id,
            context_snapshot_id: request.scope.context_snapshot_id,
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
    );
}
fn list(db: &Connection, manuscript_id: Uuid) -> Page {
    repository::list(
        db,
        Request {
            manuscript_id,
            after: None,
            limit: 64,
        },
    )
    .expect("scoped pending list")
}
#[test]
fn durable_delivery_survives_database_reopen_and_is_not_created_by_history_reads() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("db.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let request = setup(&mut db, 113);
    assert!(list(&db, request.scope.manuscript_id).items.is_empty());
    start(&db, &request);
    append(
        &db,
        &request,
        2,
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    assert_eq!(list(&db, request.scope.manuscript_id).items.len(), 1);
    assert_eq!(list(&db, Uuid::new_v4()).items.len(), 0);
    drop(db);
    let db = Connection::open(path).unwrap();
    assert_eq!(
        list(&db, request.scope.manuscript_id).items[0].turn_id,
        request.turn_id
    );
    assert!(repository::acknowledge(&db, &request, false).is_err());
    // Only the command's independently verified empty copy can settle this case.
    repository::acknowledge(&db, &request, true).expect("empty native result");
    repository::acknowledge(&db, &request, true).expect("idempotent acknowledgement");
    assert!(list(&db, request.scope.manuscript_id).items.is_empty());
}
#[test]
fn migration_does_not_register_old_prepared_or_completed_workspaces() {
    let mut db = Connection::open_in_memory().unwrap();
    let request = setup(&mut db, 112);
    start(&db, &request);
    append(
        &db,
        &request,
        2,
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    db.execute_batch(include_str!(
        "../../../../migrations/0113_writing_agent_file_deliveries.sql"
    ))
    .unwrap();
    assert!(list(&db, request.scope.manuscript_id).items.is_empty());
}
#[test]
fn acknowledgements_require_exact_admission_and_terminal_proof() {
    let mut db = Connection::open_in_memory().unwrap();
    let request = setup(&mut db, 113);
    start(&db, &request);
    assert!(repository::acknowledge(&db, &request, true).is_err());
    append(
        &db,
        &request,
        2,
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: TurnOutcome::Interrupted,
        },
    );
    let mut wrong = request.clone();
    wrong.scope.context_snapshot_id = Uuid::new_v4();
    assert!(repository::acknowledge(&db, &wrong, true).is_err());
    wrong = request.clone();
    wrong.scope.manuscript_id = Uuid::new_v4();
    assert!(repository::acknowledge(&db, &wrong, true).is_err());
    assert!(repository::settlement_proven(&db, &request).expect("interrupted admission"));
    repository::acknowledge(&db, &request, false)
        .expect("terminal no automatic partial application");
    assert!(list(&db, request.scope.manuscript_id).items.is_empty());
}
#[test]
fn discovery_rejects_corrupt_envelopes_and_ignores_archived_or_unbound_hosts() {
    let mut db = Connection::open_in_memory().unwrap();
    let request = setup(&mut db, 113);
    start(&db, &request);
    db.execute("update agent_conversations set archived=1", [])
        .unwrap();
    assert!(list(&db, request.scope.manuscript_id).items.is_empty());
    db.execute("update agent_conversations set archived=0", [])
        .unwrap();
    db.execute(
        "update agent_items set payload_json=json_set(payload_json,'$.seq',0)",
        [],
    )
    .unwrap();
    assert!(
        repository::list(
            &db,
            Request {
                manuscript_id: request.scope.manuscript_id,
                after: None,
                limit: 64
            }
        )
        .is_err()
    );
    db.execute("delete from agent_conversation_bindings", [])
        .unwrap();
    assert!(list(&db, request.scope.manuscript_id).items.is_empty());
}
#[test]
fn registration_rolls_back_with_workspace_and_requests_cannot_execute_or_forge_acknowledgement() {
    let mut db = Connection::open_in_memory().unwrap();
    let request = setup(&mut db, 113);
    db.execute("delete from agent_execution_workspaces", [])
        .unwrap();
    let tx = db.transaction().unwrap();
    workspace(&tx, &request);
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from writing_agent_file_deliveries",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let bad = serde_json::json!({"scope":request.scope,"turnId":request.turn_id,"emptyCopyVerified":true});
    assert!(serde_json::from_value::<Acknowledge>(bad).is_err());
    let bad = serde_json::json!({"manuscriptId":request.scope.manuscript_id,"after":null,"limit":64,"start":true});
    assert!(serde_json::from_value::<Request>(bad).is_err());
}

#[test]
fn pagination_continues_after_an_earlier_delivery_is_settled() {
    let mut db = Connection::open_in_memory().unwrap();
    let first = setup(&mut db, 113);
    start(&db, &first);
    append(
        &db,
        &first,
        2,
        EventPayload::TurnFinished {
            turn_id: first.turn_id,
            outcome: TurnOutcome::Interrupted,
        },
    );
    let mut second = first.clone();
    second.scope.request_id = Uuid::new_v4();
    second.scope.context_snapshot_id = Uuid::new_v4();
    second.turn_id = Uuid::new_v4();
    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')", params![second.scope.conversation_id.to_string(),second.scope.request_id.to_string(),second.turn_id.to_string(),second.scope.context_snapshot_id.to_string()]).unwrap();
    workspace(&db, &second);
    append(
        &db,
        &second,
        3,
        EventPayload::TurnStarted {
            task_kind: research_agent_core::protocol::TaskKind::Regular,
            turn_id: second.turn_id,
            context_snapshot_id: second.scope.context_snapshot_id,
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
    );
    let page = repository::list(
        &db,
        Request {
            manuscript_id: first.scope.manuscript_id,
            after: None,
            limit: 1,
        },
    )
    .unwrap();
    assert_eq!(page.items[0].turn_id, first.turn_id);
    assert_eq!(page.next_cursor, Some(page.items[0].id));
    repository::acknowledge(&db, &first, false).unwrap();
    let next = repository::list(
        &db,
        Request {
            manuscript_id: first.scope.manuscript_id,
            after: page.next_cursor,
            limit: 1,
        },
    )
    .unwrap();
    assert_eq!(next.items.len(), 1);
    assert_eq!(next.items[0].turn_id, second.turn_id);
    assert!(next.items[0].id > page.items[0].id);
    assert_eq!(next.next_cursor, None);
}

#[test]
fn terminal_confirmation_does_not_use_an_older_lease_when_latest_has_no_proposal() {
    let mut db = Connection::open_in_memory().unwrap();
    let request = setup(&mut db, 229);
    start(&db, &request);
    let scope = &request.scope;
    let original: String = db.query_row("select lease_id from agent_execution_workspaces where conversation_id=?1 and request_id=?2", params![scope.conversation_id.to_string(), scope.request_id.to_string()], |row| row.get(0)).unwrap();
    let proof = Uuid::new_v4();
    db.execute("insert into writing_agent_delivery_proofs(delivery_id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,proof_json) values(?1,?2,?3,?4,?5,?6,?7,?8,'{}')",
        params![proof.to_string(),scope.conversation_id.to_string(),scope.request_id.to_string(),request.turn_id.to_string(),scope.context_snapshot_id.to_string(),scope.manuscript_id.to_string(),original,"0".repeat(64)]).unwrap();
    db.execute("insert into writing_agent_delivery_tool_calls(delivery_id,item_id,call_id) values(?1,?2,?3)", params![proof.to_string(),Uuid::new_v4().to_string(),Uuid::new_v4().to_string()]).unwrap();
    db.execute(
        "insert into writing_agent_delivery_results(delivery_id,result_item_id) values(?1,?2)",
        params![proof.to_string(), Uuid::new_v4().to_string()],
    )
    .unwrap();
    let successor = Uuid::new_v4();
    db.execute("insert into writing_agent_workspace_generations(conversation_id,request_id,generation,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json,predecessor_delivery_id,state) values(?1,?2,1,?3,?4,?5,?6,?7,'{}',?8,'revoked')",
        params![scope.conversation_id.to_string(),scope.request_id.to_string(),request.turn_id.to_string(),scope.context_snapshot_id.to_string(),scope.manuscript_id.to_string(),successor.to_string(),"1".repeat(64),proof.to_string()]).unwrap();
    db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version,status) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,'pending')",
        params![Uuid::new_v4().to_string(),scope.conversation_id.to_string(),scope.request_id.to_string(),request.turn_id.to_string(),scope.context_snapshot_id.to_string(),scope.manuscript_id.to_string(),original,"0".repeat(64),"f".repeat(64)]).unwrap();
    append(
        &db,
        &request,
        2,
        EventPayload::TurnFinished {
            turn_id: request.turn_id,
            outcome: TurnOutcome::Completed,
        },
    );
    assert_eq!(
        repository::latest_binding(&db, &request).unwrap().lease_id,
        successor
    );
    assert!(!repository::settlement_proven(&db, &request).unwrap());
    assert!(repository::acknowledge(&db, &request, false).is_err());
}
