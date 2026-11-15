use super::*;
use research_agent_core::{
    protocol::{ModelIdentity, TaskKind, TurnOutcome},
    submission::SubmissionInput,
};

fn fixture() -> (Connection, Uuid, Uuid, Uuid) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let child = Uuid::new_v4();
    let request = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Child')",
        [child.to_string()],
    )
    .unwrap();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        text: "Synthetic child".into(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "test".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
    };
    let tx = db.transaction().unwrap();
    let record =
        super::super::repository::submissions::insert_in(&tx, child, request, input).unwrap();
    tx.commit().unwrap();
    (db, child, request, record.turn_id)
}

#[test]
fn cleanup_without_constructed_command_does_not_invent_an_archive_intent() {
    let child = Uuid::new_v4();
    let failure = ArchiveFailure {
        request_id: Uuid::new_v4(),
        archive: None,
        reason: research_agent_core::conversation::CatalogMutationError::Domain(
            CatalogError::Unavailable,
        )
        .into(),
    };
    let diagnostic = match unresolved_tool_error(&failure, child) {
        research_agent_core::runner::RunError::ToolOutcomeUnknownDiagnostic(value) => {
            serde_json::to_value(value).unwrap()
        }
        other => panic!("expected recovery diagnostic, got {other:?}"),
    };
    assert_eq!(diagnostic["resultId"], child.to_string());
    assert!(diagnostic.get("childArchiveIntentVersion").is_none());
    assert!(diagnostic.get("childArchiveOperationId").is_none());
    assert!(diagnostic.get("childArchiveExpectedWatermark").is_none());
}
fn append(db: &mut Connection, child: Uuid, request: Uuid, turn: Uuid, first: u64, finished: bool) {
    let payload = if finished {
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Failed,
        }
    } else {
        EventPayload::TurnStarted {
            turn_id: turn,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "test".into(),
                revision: None,
            },
        }
    };
    let event =
        AgentEvent::new(ConversationId::from_uuid(child), first, 0, request, payload).unwrap();
    super::super::repository::append_batch(
        db,
        ConversationId::from_uuid(child),
        Uuid::new_v4(),
        first - 1,
        &[event],
    )
    .unwrap();
}

#[test]
fn original_submission_terminal_is_frozen_even_when_later_turns_advance_current_head() {
    let (mut db, child, request, turn) = fixture();
    assert_eq!(watermark(&db, child, request), Ok(0));
    append(&mut db, child, request, turn, 1, false);
    assert_eq!(watermark(&db, child, request), Err(CatalogError::Busy));
    append(&mut db, child, request, turn, 2, true);
    assert_eq!(watermark(&db, child, request), Ok(2));
    let later = Uuid::new_v4();
    append(&mut db, child, Uuid::new_v4(), later, 3, false);
    append(&mut db, child, Uuid::new_v4(), later, 4, true);
    assert_eq!(watermark(&db, child, request), Ok(2));
    assert_eq!(watermark(&db, child, Uuid::new_v4()), Ok(0));
}

#[test]
fn malformed_frozen_submission_is_rejected_instead_of_reinterpreted_as_w0() {
    let (db, child, request, _) = fixture();
    db.execute("update agent_submissions set input_json='{}'", [])
        .unwrap();
    assert_eq!(watermark(&db, child, request), Err(CatalogError::Corrupt));
}

#[cfg(all(debug_assertions, feature = "development-data"))]
#[tokio::test]
async fn preview_uses_original_terminal_watermark_before_mocking_archive() {
    use crate::data_interface::Database;
    let root = tempfile::tempdir().unwrap();
    let (mut sql, child, request, turn) = fixture();
    append(&mut sql, child, request, turn, 1, false);
    append(&mut sql, child, request, turn, 2, true);
    let db = Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let failure = preview_archive_unconfirmed(&store, &catalog, child, request)
        .await
        .unwrap_err();
    assert_eq!(
        failure.archive.as_ref().unwrap().expected_watermark,
        Some(2)
    );
    let saved = db.inspect_child_cleanup(child).unwrap().unwrap();
    assert_eq!(saved.intent.archive.expected_watermark, Some(2));
    assert_eq!(
        saved.intent.archive.operation_id,
        failure.archive.unwrap().operation_id
    );
    assert!(
        !catalog
            .read(ConversationId::from_uuid(child), None)
            .await
            .unwrap()
            .archived
    );
}

#[tokio::test]
async fn failed_preparation_never_dispatches_archive() {
    use crate::data_interface::Database;
    let root = tempfile::tempdir().unwrap();
    let mut sql = Connection::open(root.path().join("prepare.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let child = Uuid::new_v4();
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,'Synthetic child')",
        [child.to_string()],
    )
    .unwrap();
    sql.execute_batch("create trigger fixture_reject_cleanup_prepare before insert on agent_child_cleanup_intents begin select raise(abort, 'synthetic preparation failure'); end;").unwrap();
    let db = Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let request = Uuid::new_v4();
    let failure = archive(&store, &catalog, child, request).await.unwrap_err();
    assert!(failure.archive.is_none());
    let diagnostic = match unresolved_tool_error(&failure, child) {
        research_agent_core::runner::RunError::ToolOutcomeUnknownDiagnostic(value) => {
            serde_json::to_value(value).unwrap()
        }
        other => panic!("expected preparation diagnostic, got {other:?}"),
    };
    assert_eq!(diagnostic["code"], "DATA_SQL_UNAVAILABLE");
    assert_eq!(
        diagnostic["journalRequestId"],
        crate::data_interface::ChildCleanupIntent::preparation_id(child, request).to_string()
    );
    assert!(diagnostic.get("childArchiveIntentVersion").is_none());
    let sql = db.lock().unwrap();
    assert!(
        !sql.query_row(
            "select archived from agent_conversations where id=?1",
            [child.to_string()],
            |row| row.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert_eq!(
        sql.query_row(
            "select count(*) from agent_child_cleanup_intents",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(sql.query_row("select count(*) from data_sql_operation_receipts where kind='agent.conversation.archive'", [], |row| row.get::<_, u64>(0)).unwrap(), 0);
}

#[tokio::test]
async fn cleanup_unknown_retains_original_request_and_phase_for_separate_reopen_inspection() {
    use crate::data_interface::Database;
    use research_agent_core::conversation::ArchiveStatus;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("child.sqlite3");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let child = Uuid::new_v4();
    let request = Uuid::new_v4();
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,'Synthetic child')",
        [child.to_string()],
    )
    .unwrap();
    sql.execute_batch("create table child_archive_parent(id integer primary key); create table child_archive_deferred(id integer references child_archive_parent(id) deferrable initially deferred); create trigger reject_child_archive_commit after insert on data_sql_operation_receipts when new.kind='agent.conversation.archive' begin insert into child_archive_deferred(id) values(9); end;").unwrap();
    let db = Database::open(sql, &root.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
    let catalog = SqliteConversationCatalog::new(db.clone());
    let failure = archive(&store, &catalog, child, request).await.unwrap_err();
    let diagnostic = match unresolved_tool_error(&failure, child) {
        research_agent_core::runner::RunError::ToolOutcomeUnknownDiagnostic(value) => value,
        other => panic!("expected recovery diagnostic, got {other:?}"),
    };
    let detail = serde_json::to_value(diagnostic).unwrap();
    assert_eq!(detail["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(detail["requestId"], request.to_string());
    assert_eq!(detail["resultId"], child.to_string());
    assert_eq!(detail["childArchiveIntentVersion"], 1);
    assert_eq!(detail["childArchiveExpectedWatermark"], 0);
    assert_eq!(
        detail["childArchiveOperationId"],
        failure.archive.as_ref().unwrap().operation_id.to_string()
    );
    assert_eq!(
        detail["journalRequestId"],
        failure.archive.as_ref().unwrap().operation_id.to_string()
    );
    assert!(matches!(
        unresolved_tool_error(&failure, Uuid::new_v4()),
        research_agent_core::runner::RunError::ToolOutcomeUnknown
    ));
    let wire = serde_json::to_value(failure).unwrap();
    assert_eq!(wire["requestId"], request.to_string());
    assert_eq!(wire["archive"]["expectedWatermark"], 0);
    assert_eq!(wire["reason"]["reason"]["code"], "DATA_SQL_COMMIT_UNKNOWN");
    let original = ArchiveConversation {
        operation_id: serde_json::from_value(wire["archive"]["operationId"].clone()).unwrap(),
        id: ConversationId::from_uuid(child),
        archived: true,
        expected_watermark: Some(0),
    };
    assert_ne!(original.operation_id, child);
    assert!(matches!(
        catalog.archive_status(original.clone()).await.unwrap(),
        ArchiveStatus::Unconfirmed { blocked: true, .. }
    ));
    assert!(db.lock().is_err());
    let observer = Connection::open(&path).unwrap();
    assert!(
        !observer
            .query_row("select archived from agent_conversations", [], |row| row
                .get::<_, bool>(
                0
            ))
            .unwrap()
    );
    observer
        .execute_batch("drop trigger reject_child_archive_commit;")
        .unwrap();
    drop(observer);
    drop(catalog);
    drop(store);
    drop(db);
    let reopened = Database::open(
        Connection::open(path).unwrap(),
        &root.path().join("data-log"),
    )
    .unwrap();
    let catalog = SqliteConversationCatalog::new(reopened.clone());
    assert!(matches!(
        catalog.archive_status(original.clone()).await.unwrap(),
        ArchiveStatus::NoSqlReceipt { .. }
    ));
    assert!(!catalog.read(original.id, None).await.unwrap().archived);
    // Inspection never retries. A separate original-intent command is explicit.
    catalog.set_archived(original).await.unwrap();
    reopened.shutdown().unwrap();
}
