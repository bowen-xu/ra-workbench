use super::*;
use research_agent_core::protocol::ModelIdentity;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, ChildSubmissionIntent) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("data.sqlite3");
    let mut sql = rusqlite::Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let parent = Uuid::new_v4();
    let child = Uuid::new_v4();
    sql.execute("insert into note_pages(id,title,created_at,updated_at) values('fixture','fixture','now','now')", []).unwrap();
    for id in [parent, child] {
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'fixture')",
            [id.to_string()],
        )
        .unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','fixture')", [id.to_string()]).unwrap();
    }
    let intent = ChildSubmissionIntent {
        parent,
        child,
        request: child_submission_request_id(child),
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "fixture".into(),
        },
        input: SubmissionInput {
            remote_compaction_v2: None,
            text: "synthetic".into(),
            user_input: vec![],
            skill_instructions: vec![],
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "test".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
        },
    };
    (root, path, intent)
}

fn cleanup(intent: &ChildSubmissionIntent) -> crate::data_interface::ChildCleanupIntent {
    let mut phase = b"agent.child.cleanup.archive:v1:".to_vec();
    phase.extend_from_slice(intent.request.as_bytes());
    crate::data_interface::ChildCleanupIntent {
        request_id: intent.request,
        archive: research_agent_core::conversation::ArchiveConversation {
            operation_id: Uuid::new_v5(&intent.child, &phase),
            id: research_agent_core::conversation::ConversationId::from_uuid(intent.child),
            archived: true,
            expected_watermark: Some(0),
        },
    }
}

#[test]
fn unsubmitted_cleanup_and_submission_exclude_each_other() {
    for cleanup_first in [true, false] {
        let (root, path, intent) = fixture();
        let db = Database::open(
            rusqlite::Connection::open(path).unwrap(),
            &root.path().join("log"),
        )
        .unwrap();
        let cleanup = cleanup(&intent);
        if cleanup_first {
            db.prepare_unsubmitted_child_cleanup(&cleanup).unwrap();
            assert!(matches!(
                db.prepare_child_submission(&intent),
                Err(SqlImmediateFailure::Conflict(_))
            ));
            let mut connection = db.lock().unwrap();
            let tx = connection.transaction().unwrap();
            assert!(matches!(
                submissions::insert_in(&tx, intent.child, Uuid::new_v4(), intent.input.clone()),
                Err(SubmissionError::Conflict)
            ));
            assert!(matches!(
                submissions::insert_in_with_images(
                    &tx,
                    intent.child,
                    Uuid::new_v4(),
                    intent.input.clone(),
                    &[]
                ),
                Err(SubmissionError::Conflict)
            ));
        } else {
            db.prepare_child_submission(&intent).unwrap();
            assert!(matches!(
                db.prepare_unsubmitted_child_cleanup(&cleanup),
                Err(SqlImmediateFailure::Conflict(_))
            ));
            assert!(db.inspect_child_cleanup(intent.child).unwrap().is_none());
            db.prepare_child_cleanup(&cleanup).unwrap();
            let mut connection = db.lock().unwrap();
            let tx = connection.transaction().unwrap();
            assert!(matches!(
                submissions::insert_in_with_images(
                    &tx,
                    intent.child,
                    intent.request,
                    intent.input.clone(),
                    &[],
                ),
                Err(SubmissionError::Conflict)
            ));
        }
    }
}

#[test]
fn missing_cleanup_table_after_migration_rejects_shared_submission() {
    let (root, path, intent) = fixture();
    let db = Database::open(
        rusqlite::Connection::open(path).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let mut connection = db.lock().unwrap();
    connection
        .execute_batch("drop table agent_child_cleanup_intents")
        .unwrap();
    let tx = connection.transaction().unwrap();
    assert!(matches!(
        submissions::insert_in(&tx, intent.child, intent.request, intent.input),
        Err(SubmissionError::Unavailable)
    ));
}

#[test]
fn concurrent_unsubmitted_cleanup_and_submission_have_one_winner() {
    let (root, path, intent) = fixture();
    let db = Database::open(
        rusqlite::Connection::open(path).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let cleanup = cleanup(&intent);
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writer = db.clone();
    let other_gate = gate.clone();
    let writer_intent = intent.clone();
    let thread = std::thread::spawn(move || {
        other_gate.wait();
        writer.prepare_child_submission(&writer_intent)
    });
    gate.wait();
    let cleaned = db.prepare_unsubmitted_child_cleanup(&cleanup);
    let submitted = thread.join().unwrap();
    assert_ne!(cleaned.is_ok(), submitted.is_ok());
    if cleaned.is_ok() {
        assert!(matches!(submitted, Err(SqlImmediateFailure::Conflict(_))));
    } else {
        assert!(matches!(cleaned, Err(SqlImmediateFailure::Conflict(_))));
    }
}

#[test]
fn child_submission_and_receipt_are_atomic_and_exactly_replayed() {
    let (root, path, intent) = fixture();
    let db = Database::open(
        rusqlite::Connection::open(path).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let original = db.prepare_child_submission(&intent).unwrap();
    assert!(!original.replayed);
    let replay = db.prepare_child_submission(&intent).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.record, original.record);
    let mut changed = intent.clone();
    changed.input.text = "different".into();
    assert!(matches!(
        db.prepare_child_submission(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    let sql = db.lock().unwrap();
    let counts: (u64,u64) = sql.query_row("select (select count(*) from agent_submissions), (select count(*) from data_sql_operation_receipts where kind='agent.child.submission')", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(counts, (1, 1));
    sql.execute(
        "delete from agent_conversation_bindings where conversation_id=?1",
        [intent.child.to_string()],
    )
    .unwrap();
    drop(sql);
    assert!(matches!(
        db.prepare_child_submission(&intent),
        Err(SqlImmediateFailure::Conflict(_))
    ));
}

#[test]
fn lost_acknowledgement_blocks_until_reopen_and_retains_original_record() {
    for (fault, committed) in [
        (super::super::Fault::BeforeCommit, false),
        (super::super::Fault::AfterCommit, true),
    ] {
        let (root, path, intent) = fixture();
        let log = root.path().join("log");
        let db = Database::open(rusqlite::Connection::open(&path).unwrap(), &log).unwrap();
        assert!(matches!(
            db.prepare_child_submission_owned(&intent, false, fault),
            Err(SqlImmediateFailure::Unknown(_))
        ));
        assert!(matches!(
            db.prepare_child_submission(&intent),
            Err(SqlImmediateFailure::RecoveryRequired(_))
        ));
        drop(db);
        let db = Database::open(rusqlite::Connection::open(&path).unwrap(), &log).unwrap();
        let sql = db.lock().unwrap();
        let counts: (u64,u64) = sql.query_row("select (select count(*) from agent_submissions), (select count(*) from data_sql_operation_receipts where kind='agent.child.submission')", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(counts, if committed { (1, 1) } else { (0, 0) });
        drop(sql);
        let record = db.prepare_child_submission(&intent).unwrap();
        assert_eq!(record.record.request_id, intent.request);
        assert_eq!(record.replayed, committed);
    }
}

#[tokio::test]
async fn replayed_submission_requires_reconciliation_before_host_dispatch() {
    use research_agent_core::conversation::{CatalogDataFailureKind, CatalogMutationError};
    let (root, path, intent) = fixture();
    let db = Database::open(
        rusqlite::Connection::open(path).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let original = db.prepare_child_submission(&intent).unwrap();
    let catalog = crate::agent::repository::SqliteConversationCatalog::new(db.clone());
    let error = catalog
        .prepare_child_submission(intent.clone())
        .await
        .unwrap_err();
    assert!(
        matches!(error, CatalogMutationError::Data(value) if value.kind == CatalogDataFailureKind::Unknown && value.diagnostic.safe_cause == "child_submission_requires_reconciliation")
    );
    assert_eq!(
        db.prepare_child_submission(&intent).unwrap().record,
        original.record
    );
}

#[test]
fn replay_rechecks_original_resource_before_returning_a_record() {
    let (root, path, intent) = fixture();
    let sql = rusqlite::Connection::open(path).unwrap();
    crate::notes::register_projection_functions(&sql).unwrap();
    crate::storage::schema_compat::register_functions(&sql).unwrap();
    sql.execute_batch("pragma foreign_keys=on;").unwrap();
    let db = Database::open(sql, &root.path().join("log")).unwrap();
    db.prepare_child_submission(&intent).unwrap();
    db.lock()
        .unwrap()
        .execute("delete from note_pages where id='fixture'", [])
        .unwrap();
    assert!(matches!(
        db.prepare_child_submission(&intent),
        Err(SqlImmediateFailure::Conflict(_))
    ));
}

#[tokio::test]
async fn legacy_submission_without_original_receipt_is_not_dispatched() {
    use research_agent_core::conversation::{CatalogDataFailureKind, CatalogMutationError};
    let (root, path, intent) = fixture();
    let db = Database::open(
        rusqlite::Connection::open(path).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    {
        let mut sql = db.lock().unwrap();
        let tx = sql.transaction().unwrap();
        crate::agent::repository::submissions::insert_in(
            &tx,
            intent.child,
            intent.request,
            intent.input.clone(),
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let catalog = crate::agent::repository::SqliteConversationCatalog::new(db.clone());
    let error = catalog.prepare_child_submission(intent).await.unwrap_err();
    assert!(
        matches!(error, CatalogMutationError::Data(value) if value.kind == CatalogDataFailureKind::Conflict && value.diagnostic.safe_cause == "legacy_child_submission_without_receipt")
    );
}

#[test]
fn archived_parent_or_child_cannot_admit_submission() {
    for archive_parent in [false, true] {
        let (root, path, intent) = fixture();
        let db = Database::open(
            rusqlite::Connection::open(path).unwrap(),
            &root.path().join("log"),
        )
        .unwrap();
        let id = if archive_parent {
            intent.parent
        } else {
            intent.child
        };
        db.lock()
            .unwrap()
            .execute(
                "update agent_conversations set archived=1 where id=?1",
                [id.to_string()],
            )
            .unwrap();
        assert!(matches!(
            db.prepare_child_submission(&intent),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        let sql = db.lock().unwrap();
        let count: u64 = sql
            .query_row("select count(*) from agent_submissions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[cfg(all(debug_assertions, feature = "development-data"))]
#[test]
fn preview_failure_uses_the_owned_transaction_and_does_not_persist_a_submission() {
    let (root, path, intent) = fixture();
    let db = Database::open(
        rusqlite::Connection::open(path).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    assert!(matches!(
        db.preview_child_submission_failure(&intent),
        Err(SqlImmediateFailure::Unavailable(_))
    ));
    let sql = db.lock().unwrap();
    let counts: (u64,u64) = sql.query_row("select (select count(*) from agent_submissions), (select count(*) from data_sql_operation_receipts where kind='agent.child.submission')", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(counts, (0, 0));
}
