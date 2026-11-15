use super::super::Fault;
use super::*;
use crate::data_interface::{SharedDatabase, journal::JournalCommand};
use research_agent_core::{
    conversation::{BindingKind, CatalogError, ConversationBinding, ConversationId},
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use research_data_runtime::WriteOptions;
use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, mpsc},
};

#[path = "tests/failures.rs"]
mod failures;

fn open(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("rename.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("data-log")).unwrap()
}
fn fixture() -> (tempfile::TempDir, SharedDatabase, RenameConversation) {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path());
    let command = RenameConversation {
        operation_id: Uuid::new_v4(),
        id: ConversationId::from_uuid(Uuid::new_v4()),
        expected_title: "A".into(),
        title: "B".into(),
    };
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title) values(?1,'A')",
            [command.id.as_uuid().to_string()],
        )
        .unwrap();
    (dir, db, command)
}
fn metadata(db: &Connection, command: &RenameConversation) -> (String, String) {
    db.query_row(
        "select title,updated_at from agent_conversations where id=?1",
        [command.id.as_uuid().to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}
fn receipt_count(db: &Connection) -> u64 {
    db.query_row(
        "select count(*) from data_sql_operation_receipts",
        [],
        |row| row.get(0),
    )
    .unwrap()
}
fn pending(command: &RenameConversation, batch: Uuid) -> JournalCommand {
    let event = AgentEvent::new(
        command.id,
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
    JournalCommand::new(command.id, batch, 0, vec![event], None, 1_790_000_000_000).unwrap()
}

#[test]
fn zero_fence_proof_replays_without_reverting_a_later_rename() {
    let (_dir, db, first) = fixture();
    let proof = db.rename_conversation(&first).unwrap();
    assert_eq!(proof.sql_fence.position, 0);
    assert!(!proof.replayed);
    let second = RenameConversation {
        operation_id: Uuid::new_v4(),
        expected_title: "B".into(),
        title: "A".into(),
        ..first.clone()
    };
    let second_proof = db.rename_conversation(&second).unwrap();
    assert_eq!(second_proof.sql_fence, proof.sql_fence);
    let before = metadata(&db.raw.lock().unwrap(), &first);
    // Any accidental mutation on replay now fails; current title need not equal
    // the historical target in the proof.
    db.raw.lock().unwrap().execute_batch("create trigger deny_rename_replay before update of title on agent_conversations begin select raise(abort,'unexpected rename'); end;").unwrap();
    let replay = db.rename_conversation(&first).unwrap();
    assert_eq!(
        replay,
        SqlImmediateReceipt {
            replayed: true,
            ..proof.clone()
        }
    );
    assert_eq!(metadata(&db.raw.lock().unwrap(), &first), before);
    assert_eq!(before.0, "A");
    let sql = db.raw.lock().unwrap();
    assert_eq!(receipt_count(&sql), 2);
    assert_eq!(sql.query_row("select kind,version,result_json from data_sql_operation_receipts where operation_id=?1",
        [first.operation_id.to_string()], |row| Ok((row.get::<_,String>(0)?,row.get::<_,u32>(1)?,row.get::<_,String>(2)?))).unwrap(),
        (KIND.into(), VERSION, "null".into()));
    assert_eq!(
        sql.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn status_does_not_drain_pending_and_rename_materializes_before_its_transaction() {
    let (dir, db, command) = fixture();
    db.write(pending(&command, Uuid::new_v4()), WriteOptions::default())
        .unwrap();
    let observer = Connection::open(dir.path().join("rename.sqlite3")).unwrap();
    let durable = || {
        observer
            .query_row("select durable_count from agent_conversations", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap()
    };
    assert_eq!(durable(), 0);
    assert!(matches!(
        db.rename_status(&command).unwrap(),
        SqlImmediateStatus::NoSqlReceipt {
            sql_fence: ReadFence { position: 0, .. },
            ..
        }
    ));
    assert_eq!(durable(), 0);
    let proof = db.rename_conversation(&command).unwrap();
    assert_eq!(proof.sql_fence.position, 1);
    assert_eq!(durable(), 1);
    assert_eq!(metadata(&observer, &command).0, "B");
    assert!(
        matches!(db.rename_status(&command).unwrap(),SqlImmediateStatus::Committed(value) if value.fingerprint==proof.fingerprint)
    );
    assert_eq!(
        db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .accepted_position,
        1
    );
}

#[test]
fn changed_intent_and_foreign_family_conflict_without_mutation() {
    let (_dir, db, command) = fixture();
    let proof = db.rename_conversation(&command).unwrap();
    for changed in [
        RenameConversation {
            title: " B ".into(),
            ..command.clone()
        },
        RenameConversation {
            expected_title: "B".into(),
            ..command.clone()
        },
        RenameConversation {
            id: ConversationId::from_uuid(Uuid::new_v4()),
            ..command.clone()
        },
    ] {
        assert!(matches!(
            db.rename_conversation(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert!(matches!(
            db.rename_status(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
    }
    // Keep the rollback family's pre-lookup live checks meaningful.
    let auth = super::super::AuthorizeRollback {
        operation_id: command.operation_id,
        conversation_id: command.id,
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "rename-note".into(),
        },
    };
    {
        let sql = db.lock().unwrap();
        sql.execute("insert into note_pages(id,title,created_at,updated_at) values('rename-note','Note','now','now')",[]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','rename-note')",[command.id.as_uuid().to_string()]).unwrap();
    }
    assert!(matches!(
        db.authorize_rollback(&auth),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        db.rollback_authorization_status(&auth),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(
        matches!(db.rename_status(&command).unwrap(),SqlImmediateStatus::Committed(value) if value.fingerprint==proof.fingerprint)
    );
    assert_eq!(metadata(&db.raw.lock().unwrap(), &command).0, "B");
    assert_eq!(receipt_count(&db.raw.lock().unwrap()), 1);
}

#[test]
fn journal_derived_identity_conflicts_but_raw_batch_and_request_ids_are_separate() {
    let (_dir, db, command) = fixture();
    db.write(
        pending(&command, command.operation_id),
        WriteOptions::default(),
    )
    .unwrap();
    db.rename_conversation(&command).unwrap();
    let changed = RenameConversation {
        operation_id: Uuid::new_v5(command.id.as_uuid(), command.operation_id.as_bytes()),
        ..command.clone()
    };
    assert!(matches!(
        db.rename_conversation(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(receipt_count(&db.raw.lock().unwrap()), 1);
    assert!(db.raw.lock().unwrap().is_autocommit());
}

#[test]
fn fresh_cas_is_required_even_when_the_target_is_already_current_and_noop_keeps_time() {
    let (_dir, db, command) = fixture();
    let unchanged = RenameConversation {
        title: " A ".into(),
        ..command.clone()
    };
    let original = metadata(&db.raw.lock().unwrap(), &command);
    db.rename_conversation(&unchanged).unwrap();
    assert_eq!(metadata(&db.raw.lock().unwrap(), &command), original);
    let conflict = RenameConversation {
        operation_id: Uuid::new_v4(),
        expected_title: "wrong".into(),
        title: "A".into(),
        ..command.clone()
    };
    assert!(matches!(
        db.rename_conversation(&conflict),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(receipt_count(&db.raw.lock().unwrap()), 1);
    assert!(matches!(
        db.rename_status(&conflict).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
}

#[test]
fn stored_padded_expected_title_is_opaque_and_invalid_targets_do_not_write() {
    let (_dir, db, mut command) = fixture();
    // The deployed SQL check bounds trim(title), not the raw stored value.
    command.expected_title = format!("{}Legacy{}", " ".repeat(200), " ".repeat(200));
    db.raw
        .lock()
        .unwrap()
        .execute(
            "update agent_conversations set title=?1",
            [&command.expected_title],
        )
        .unwrap();
    db.rename_conversation(&command).unwrap();
    for invalid in ["".to_owned(), " ".into(), "x\0y".into(), "🦀".repeat(181)] {
        let changed = RenameConversation {
            operation_id: Uuid::new_v4(),
            expected_title: "B".into(),
            title: invalid,
            ..command.clone()
        };
        assert!(matches!(
            db.rename_conversation(&changed),
            Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput))
        ));
    }
    assert_eq!(metadata(&db.raw.lock().unwrap(), &command).0, "B");
    assert_eq!(receipt_count(&db.raw.lock().unwrap()), 1);
}

#[test]
fn replay_and_status_after_deletion_return_only_the_original_proof() {
    let (_dir, db, command) = fixture();
    let proof = db.rename_conversation(&command).unwrap();
    db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversations where id=?1",
            [command.id.as_uuid().to_string()],
        )
        .unwrap();
    assert!(db.rename_conversation(&command).unwrap().replayed);
    assert!(
        matches!(db.rename_status(&command).unwrap(),SqlImmediateStatus::Committed(value) if value.sql_fence==proof.sql_fence)
    );
    let sql = db.raw.lock().unwrap();
    assert_eq!(
        sql.query_row("select count(*) from agent_conversations", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(receipt_count(&sql), 1);
}

#[test]
fn rename_retains_the_existing_admission_until_commit_before_a_journal_writer() {
    let (_dir, db, command) = fixture();
    let (entered, enter) = mpsc::channel();
    let (proceed, release) = mpsc::channel();
    let writer_db = Arc::clone(&db);
    let request = command.clone();
    let renaming = std::thread::spawn(move || {
        writer_db.rename_conversation_owned(
            &request,
            Fault::Pause {
                entered,
                proceed: release,
            },
        )
    });
    enter.recv().unwrap();
    let (started, start) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let journal_db = Arc::clone(&db);
    let record = pending(&command, Uuid::new_v4());
    let writing = std::thread::spawn(move || {
        started.send(()).unwrap();
        let result = journal_db.write(record, WriteOptions::default());
        done.send(()).unwrap();
        result
    });
    start.recv().unwrap();
    assert!(finished.try_recv().is_err());
    proceed.send(()).unwrap();
    let proof = renaming.join().unwrap().unwrap();
    writing.join().unwrap().unwrap();
    finished.recv().unwrap();
    assert_eq!(proof.sql_fence.position, 0);
    assert_eq!(
        db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .accepted_position,
        1
    );
}
