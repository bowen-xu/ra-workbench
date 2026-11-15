use super::*;

#[tokio::test]
async fn writing_tool_original_creation_identity_survives_unknown_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let db = open(root.path());
    db.lock().unwrap().execute_batch("create table child_creation_parent(id integer primary key);
      create table child_creation_deferred(id integer references child_creation_parent(id) deferrable initially deferred);
      create trigger reject_child_creation_commit after insert on data_sql_operation_receipts when new.kind='agent.conversation.create_bound'
      begin insert into child_creation_deferred(id) values(9); end;").unwrap();
    let turn = Uuid::from_u128(71);
    let call = "provider-call-1";
    let original = intent(
        writing_tool_child_id(turn, call),
        ConversationBinding {
            kind: BindingKind::Note,
            resource_id: RESOURCE.into(),
        },
        Host::Writing,
    );
    let catalog = SqliteConversationCatalog::new(db.clone());
    let failure = create(&catalog, &original).await.unwrap_err();
    assert_eq!(failure.creation, (&original).into());
    let observer = Connection::open(root.path().join("child-creation.sqlite3")).unwrap();
    observer
        .execute_batch("drop trigger reject_child_creation_commit;")
        .unwrap();
    drop(observer);
    drop(catalog);
    drop(db);
    let reopened = open(root.path());
    let catalog = SqliteConversationCatalog::new(reopened.clone());
    let rebuilt = intent(
        writing_tool_child_id(turn, call),
        original.binding.clone(),
        Host::Writing,
    );
    assert_eq!(rebuilt, original);
    assert!(matches!(
        catalog.create_bound_status(rebuilt.clone()).await.unwrap(),
        CreateBoundStatus::NoSqlReceipt { closed: false, .. }
    ));
    let proof = create(&catalog, &rebuilt).await.unwrap();
    assert_eq!(proof.operation_id, original.operation_id);
    assert_eq!(proof.conversation_id, original.id);
    reopened.shutdown().unwrap();
}

#[tokio::test]
async fn real_commit_unknown_keeps_complete_creation_intent_for_reopen_inspection() {
    let root = tempfile::tempdir().unwrap();
    let db = open(root.path());
    db.lock().unwrap().execute_batch("create table child_creation_parent(id integer primary key);
      create table child_creation_deferred(id integer references child_creation_parent(id) deferrable initially deferred);
      create trigger reject_child_creation_commit after insert on data_sql_operation_receipts when new.kind='agent.conversation.create_bound'
      begin insert into child_creation_deferred(id) values(9); end;").unwrap();
    let catalog = SqliteConversationCatalog::new(db.clone());
    let original = command(Host::Note);
    let failure = create(&catalog, &original).await.unwrap_err();
    let wire = serde_json::to_value(failure).unwrap();
    assert_eq!(
        wire["creation"],
        json!({"operationId":original.operation_id,"id":original.id.as_uuid(),"binding":{"kind":"note","resourceId":RESOURCE}})
    );
    assert_eq!(wire["reason"]["code"], "storage");
    assert_eq!(wire["reason"]["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    let detail = &wire["reason"]["reason"]["reason"];
    assert_eq!(detail["operationId"], original.operation_id.to_string());
    assert_eq!(detail["phase"], "commit");
    assert_eq!(detail["sqliteExtendedCode"], 787);
    assert_eq!(detail["safeCause"], "sql_commit_outcome_unknown");
    let restored = restored_intent(&wire);
    assert_eq!(restored, original);
    assert!(matches!(
        catalog.create_bound_status(restored.clone()).await.unwrap(),
        CreateBoundStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.lock().is_err());
    let observer = Connection::open(root.path().join("child-creation.sqlite3")).unwrap();
    for table in [
        "agent_conversations",
        "agent_conversation_bindings",
        "data_sql_operation_receipts",
    ] {
        assert_eq!(
            observer
                .query_row(&format!("select count(*) from {table}"), [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    observer
        .execute_batch("drop trigger reject_child_creation_commit;")
        .unwrap();
    drop(observer);
    drop(catalog);
    drop(db);
    let reopened = open(root.path());
    let catalog = SqliteConversationCatalog::new(reopened.clone());
    assert!(matches!(
        catalog.create_bound_status(restored.clone()).await.unwrap(),
        CreateBoundStatus::NoSqlReceipt {
            sql_fence: 0,
            closed: false,
            ..
        }
    ));
    // Status alone did not create a row. An explicit original-intent retry is
    // a separate step, and returns the original C/op rather than a new RPC C.
    assert_eq!(
        reopened
            .lock()
            .unwrap()
            .query_row("select count(*) from agent_conversations", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    let proof = create(&catalog, &restored).await.unwrap();
    assert_eq!(proof.operation_id, original.operation_id);
    assert_eq!(proof.conversation_id, original.id);
    reopened.shutdown().unwrap();
}
