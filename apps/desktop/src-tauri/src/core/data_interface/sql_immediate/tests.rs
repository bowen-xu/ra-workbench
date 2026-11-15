use super::*;
use crate::data_interface::{IntoDatabase, SharedDatabase, journal::JournalCommand};
use research_agent_core::{
    conversation::{BindingKind, CatalogError, ConversationBinding, ConversationId},
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use research_data_runtime::WriteOptions;
use rusqlite::Connection;
use std::sync::{Arc, Mutex, mpsc};
use uuid::Uuid;

#[path = "tests/failures.rs"]
mod failures;

fn open(directory: &std::path::Path) -> SharedDatabase {
    let mut sql = Connection::open(directory.join("fixture.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &directory.join("data-log")).unwrap()
}

fn fixture() -> (tempfile::TempDir, SharedDatabase, AuthorizeRollback) {
    let directory = tempfile::tempdir().unwrap();
    let db = open(directory.path());
    let command = AuthorizeRollback {
        operation_id: Uuid::new_v4(),
        conversation_id: ConversationId::from_uuid(Uuid::new_v4()),
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "sql-note".into(),
        },
    };
    {
        let sql = db.lock().unwrap();
        sql.execute("insert into note_pages(id,title,created_at,updated_at) values('sql-note','Note','now','now'),('other-note','Other','now','now')", []).unwrap();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'SQL')",
            [command.conversation_id.as_uuid().to_string()],
        )
        .unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','sql-note'),(?1,'note','other-note')", [command.conversation_id.as_uuid().to_string()]).unwrap();
    }
    (directory, db, command)
}

fn pending(command: &AuthorizeRollback, batch_id: Uuid) -> JournalCommand {
    let event = AgentEvent::new(
        command.conversation_id,
        1,
        1,
        command.operation_id,
        EventPayload::ItemStarted {
            turn_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            kind: ItemKind::AssistantMessage,
        },
    )
    .unwrap();
    JournalCommand::new(
        command.conversation_id,
        batch_id,
        0,
        vec![event],
        None,
        1_790_000_000_000,
    )
    .unwrap()
}

fn count(db: &Connection, table: &str) -> u64 {
    match table {
        "grant" => db
            .query_row("select count(*) from agent_rollback_authority", [], |row| {
                row.get(0)
            })
            .unwrap(),
        "receipt" => db
            .query_row(
                "select count(*) from data_sql_operation_receipts",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        _ => unreachable!(),
    }
}

#[test]
fn transaction_body_panic_blocks_further_admission() {
    let (_directory, db, command) = fixture();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = db.execute_sql(
            command.operation_id,
            |_, _| panic!("synthetic transaction panic"),
            Fault::None,
        );
    }));
    assert!(result.is_err());
    assert!(
        db.write(pending(&command, Uuid::new_v4()), WriteOptions::default())
            .is_err()
    );
    assert!(db.lock().is_err());
}

#[test]
fn zero_fence_unit_receipts_replay_and_share_a_fence_without_log_positions() {
    let (_dir, db, command) = fixture();
    let first = db.authorize_rollback(&command).unwrap();
    assert_eq!(first.operation_id, command.operation_id);
    assert_eq!(first.sql_fence.position, 0);
    assert!(!first.replayed);
    let replay = db.authorize_rollback(&command).unwrap();
    assert_eq!(
        replay,
        SqlImmediateReceipt {
            replayed: true,
            ..first.clone()
        }
    );
    let mut second = command.clone();
    second.operation_id = Uuid::new_v4();
    assert_eq!(
        db.authorize_rollback(&second).unwrap().sql_fence,
        first.sql_fence
    );
    let sql = db.raw.lock().unwrap();
    assert_eq!(count(&sql, "grant"), 2);
    assert_eq!(count(&sql, "receipt"), 2);
    assert_eq!(
        sql.query_row("select count(*) from data_operation_receipts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    let results: Vec<String> = sql
        .prepare("select result_json from data_sql_operation_receipts")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(results, ["null", "null"]);
}

#[test]
fn inspection_preserves_pending_suffix_and_write_materializes_it_before_sql() {
    let (dir, db, command) = fixture();
    db.write(pending(&command, Uuid::new_v4()), WriteOptions::default())
        .unwrap();
    let observer = Connection::open(dir.path().join("fixture.sqlite3")).unwrap();
    let durable = || {
        observer
            .query_row("select durable_count from agent_conversations", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap()
    };
    assert_eq!(durable(), 0);
    assert!(matches!(
        db.rollback_authorization_status(&command).unwrap(),
        SqlImmediateStatus::NoSqlReceipt {
            sql_fence: ReadFence { position: 0, .. },
            ..
        }
    ));
    assert_eq!(durable(), 0);
    assert_eq!(count(&observer, "receipt"), 0);
    let receipt = db.authorize_rollback(&command).unwrap();
    assert_eq!(receipt.sql_fence.position, 1);
    assert_eq!(durable(), 1);
    assert!(
        matches!(db.rollback_authorization_status(&command).unwrap(), SqlImmediateStatus::Committed(value) if value.operation_id==command.operation_id)
    );
}

#[test]
fn derived_journal_identity_conflicts_but_raw_batch_and_event_request_do_not() {
    let (_dir, db, command) = fixture();
    // Both the raw batch UUID and event request UUID equal this SQL operation.
    let derived = Uuid::new_v5(
        command.conversation_id.as_uuid(),
        command.operation_id.as_bytes(),
    );
    db.write(
        pending(&command, command.operation_id),
        WriteOptions::default(),
    )
    .unwrap();
    db.authorize_rollback(&command).unwrap();
    let mut collided = command.clone();
    collided.operation_id = derived;
    assert!(matches!(
        db.authorize_rollback(&collided),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    let sql = db.raw.lock().unwrap();
    assert_eq!(count(&sql, "receipt"), 1);
    assert_eq!(count(&sql, "grant"), 1);
    assert!(sql.is_autocommit());
}

#[test]
fn replay_keeps_live_authority_checks_and_status_is_only_registration_evidence() {
    let (_dir, db, command) = fixture();
    db.authorize_rollback(&command).unwrap();
    db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [command.conversation_id.as_uuid().to_string()],
        )
        .unwrap();
    assert!(matches!(
        db.authorize_rollback(&command),
        Err(SqlImmediateFailure::Domain(CatalogError::NotFound))
    ));
    assert!(matches!(
        db.rollback_authorization_status(&command).unwrap(),
        SqlImmediateStatus::Committed(_)
    ));
    let sql = db.raw.lock().unwrap();
    assert_eq!(count(&sql, "receipt"), 1);
    assert!(sql.is_autocommit());
}

#[test]
fn changed_payload_conflicts_without_changing_the_frozen_receipt() {
    let (_dir, db, command) = fixture();
    let receipt = db.authorize_rollback(&command).unwrap();
    let mut changed = command.clone();
    changed.binding.resource_id = "other-note".into();
    assert!(matches!(
        db.authorize_rollback(&changed),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert!(matches!(
        db.rollback_authorization_status(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(
        matches!(db.rollback_authorization_status(&command).unwrap(), SqlImmediateStatus::Committed(value) if value.fingerprint==receipt.fingerprint)
    );
}

#[test]
fn facade_holds_existing_admission_until_sql_commit_before_a_journal_writer() {
    let (_dir, db, command) = fixture();
    let (entered, entry) = mpsc::channel();
    let (proceed, release) = mpsc::channel();
    let author = Arc::clone(&db);
    let auth_command = command.clone();
    let authorization = std::thread::spawn(move || {
        author.authorize_rollback_owned(
            &auth_command,
            Fault::Pause {
                entered,
                proceed: release,
            },
        )
    });
    entry
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let writer = Arc::clone(&db);
    let journal = pending(&command, Uuid::new_v4());
    let (finished, completion) = mpsc::channel();
    let (ready, readiness) = mpsc::channel();
    let task = std::thread::spawn(move || {
        ready.send(()).unwrap();
        let result = writer.write(journal, WriteOptions::default());
        finished.send(result).unwrap();
    });
    readiness
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    // This readiness observation does not prove that the scheduler has entered
    // write yet. The exact same-mutex exclusion is covered by data-runtime's
    // inspection tests and the AdmissionGuard held by this facade until commit.
    // Here the positive scope is the released transaction's W=0 receipt, the
    // following journal position 1, and inspection preserving that later suffix.
    assert!(matches!(
        completion.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    proceed.send(()).unwrap();
    assert_eq!(authorization.join().unwrap().unwrap().sql_fence.position, 0);
    assert_eq!(
        completion
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .position,
        1
    );
    task.join().unwrap();
    let saved = db.rollback_authorization_status(&command).unwrap();
    assert!(matches!(saved, SqlImmediateStatus::Committed(value) if value.sql_fence.position==0));
    // Merely inspecting the receipt has still not materialized that later log.
    assert_eq!(
        db.raw
            .lock()
            .unwrap()
            .query_row("select durable_count from agent_conversations", [], |r| r
                .get::<_, u64>(
                0
            ))
            .unwrap(),
        0
    );
}

#[test]
fn owner_none_and_migrated_schema_never_fabricate_an_epoch() {
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let raw = Arc::new(Mutex::new(sql));
    let db = Arc::clone(&raw).into_database();
    let command = AuthorizeRollback {
        operation_id: Uuid::new_v4(),
        conversation_id: ConversationId::from_uuid(Uuid::new_v4()),
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "legacy-note".into(),
        },
    };
    for error in [
        db.authorize_rollback(&command).unwrap_err(),
        db.rollback_authorization_status(&command).unwrap_err(),
    ] {
        assert!(matches!(error, SqlImmediateFailure::Unmanaged(_)));
        let metadata = error.diagnostic().unwrap();
        assert!(metadata.epoch.is_none() && metadata.sql_fence.is_none());
    }
    assert_eq!(
        raw.lock()
            .unwrap()
            .query_row("select count(*) from data_runtime_state", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn diagnostic_serialization_preserves_codes_without_sql_messages_or_bodies() {
    let (_dir, _db, command) = fixture();
    let error = rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(787),
        Some("secret body; insert into private_data".into()),
    );
    let metadata =
        SqlImmediateDiagnostic::new(&command, None, "commit", "sql_commit_outcome_unknown")
            .sqlite(&error);
    let value = serde_json::to_value(SqlImmediateFailure::Unknown(Box::new(metadata))).unwrap();
    assert_eq!(value["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(
        value["reason"]["operationId"],
        command.operation_id.to_string()
    );
    assert_eq!(value["reason"]["sqliteExtendedCode"], 787);
    assert!(!value.to_string().contains("secret body"));
    assert!(!value.to_string().contains("private_data"));
}
