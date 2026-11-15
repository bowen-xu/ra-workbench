use super::super::Fault;
use super::*;
use crate::data_interface::{SharedDatabase, journal::JournalCommand};
use research_agent_core::{
    conversation::{
        BindingKind, CatalogError, ConversationBinding, ConversationId, RenameConversation,
    },
    protocol::{AgentEvent, EventPayload, ItemKind},
};
use research_data_runtime::WriteOptions;
use rusqlite::Connection;
use std::{path::Path, sync::mpsc};

#[path = "tests/failures.rs"]
mod failures;

fn open(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("create-bound.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("data-log")).unwrap()
}
fn fixture() -> (tempfile::TempDir, SharedDatabase, CreateBoundConversation) {
    let dir = tempfile::tempdir().unwrap();
    let db = open(dir.path());
    db.lock().unwrap().execute("insert into note_pages(id,title,created_at,updated_at) values('one','One','now','now'),('two','Two','now','now')", []).unwrap();
    let command = CreateBoundConversation {
        operation_id: Uuid::new_v4(),
        id: ConversationId::from_uuid(Uuid::new_v4()),
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "one".into(),
        },
    };
    (dir, db, command)
}
fn count(db: &Connection, table: &str) -> u64 {
    // Only fixed fixture table names reach this helper; business values remain parameters.
    db.query_row(&format!("select count(*) from {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
fn records(db: &Connection) -> (u64, u64, u64) {
    (
        count(db, "agent_conversations"),
        count(db, "agent_conversation_bindings"),
        count(db, "data_sql_operation_receipts"),
    )
}
type Metadata = (String, bool, u64, String, String, String);
fn metadata(db: &Connection, id: ConversationId) -> Metadata {
    db.query_row(
        "select c.title,c.archived,c.durable_count,c.created_at,c.updated_at,b.created_at
      from agent_conversations c join agent_conversation_bindings b on b.conversation_id=c.id
      where c.id=?1 and b.resource_kind='note' and b.resource_id='one'",
        [id.as_uuid().to_string()],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )
    .unwrap()
}
fn pending(id: ConversationId, batch: Uuid) -> JournalCommand {
    let event = AgentEvent::new(
        id,
        1,
        1,
        batch,
        EventPayload::ItemStarted {
            turn_id: Uuid::new_v4(),
            item_id: Uuid::new_v4(),
            kind: ItemKind::AssistantMessage,
        },
    )
    .unwrap();
    JournalCommand::new(id, batch, 0, vec![event], None, 1_790_000_000_000).unwrap()
}
fn other_conversation(db: &SharedDatabase) -> ConversationId {
    let id = ConversationId::from_uuid(Uuid::new_v4());
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title) values(?1,'Pending')",
            [id.as_uuid().to_string()],
        )
        .unwrap();
    id
}

#[test]
fn creation_binds_and_registers_a_zero_fence_proof_with_one_frozen_time() {
    let (_dir, db, command) = fixture();
    assert_eq!(records(&db.raw.lock().unwrap()), (0, 0, 0));
    let proof = db.create_bound_conversation(&command).unwrap();
    assert!(!proof.replayed);
    assert_eq!(proof.operation_id, command.operation_id);
    assert_eq!(proof.sql_fence.position, 0);
    let sql = db.raw.lock().unwrap();
    assert_eq!(records(&sql), (1, 1, 1));
    let value = metadata(&sql, command.id);
    assert_eq!(
        (&value.0, value.1, value.2),
        (&"新对话".to_owned(), false, 0)
    );
    assert_eq!(value.3, value.4);
    assert_eq!(value.4, value.5);
    assert!(value.3.ends_with('Z'));
    assert_eq!(
        sql.query_row(
            "select kind,version,result_json from data_sql_operation_receipts",
            [],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?
            ))
        )
        .unwrap(),
        (KIND.into(), VERSION, "null".into())
    );
}

#[test]
fn replay_and_a_new_matching_intent_do_not_reset_current_metadata_or_binding_time() {
    let (_dir, db, command) = fixture();
    let proof = db.create_bound_conversation(&command).unwrap();
    db.write(pending(command.id, Uuid::new_v4()), WriteOptions::default())
        .unwrap();
    {
        let sql = db.lock().unwrap();
        sql.execute("update agent_conversations set title='Renamed',archived=1,updated_at='later' where id=?1", [command.id.as_uuid().to_string()]).unwrap();
        sql.execute_batch("create trigger deny_creation_rewrite before update on agent_conversations begin select raise(abort,'unexpected update'); end;
            create trigger deny_creation_rebind before insert on agent_conversation_bindings begin select raise(abort,'unexpected binding'); end;").unwrap();
    }
    let before = metadata(&db.raw.lock().unwrap(), command.id);
    assert_eq!(
        db.create_bound_conversation(&command).unwrap(),
        SqlImmediateReceipt {
            replayed: true,
            ..proof.clone()
        }
    );
    let second = CreateBoundConversation {
        operation_id: Uuid::new_v4(),
        ..command.clone()
    };
    assert!(!db.create_bound_conversation(&second).unwrap().replayed);
    assert_eq!(metadata(&db.raw.lock().unwrap(), command.id), before);
    assert_eq!(
        (&before.0, before.1, before.2),
        (&"Renamed".to_owned(), true, 1)
    );
    assert_eq!(
        count(&db.raw.lock().unwrap(), "data_sql_operation_receipts"),
        2
    );
    assert!(
        matches!(db.bound_conversation_creation_status(&command).unwrap(), SqlImmediateStatus::Committed(value) if value.sql_fence==proof.sql_fence)
    );
}

#[test]
fn status_does_not_materialize_pending_but_creation_preserves_its_global_order() {
    let (dir, db, command) = fixture();
    let other = other_conversation(&db);
    db.write(pending(other, Uuid::new_v4()), WriteOptions::default())
        .unwrap();
    let observer = Connection::open(dir.path().join("create-bound.sqlite3")).unwrap();
    assert!(matches!(
        db.bound_conversation_creation_status(&command).unwrap(),
        SqlImmediateStatus::NoSqlReceipt {
            sql_fence: ReadFence { position: 0, .. },
            ..
        }
    ));
    assert_eq!(
        observer
            .query_row(
                "select durable_count from agent_conversations where id=?1",
                [other.as_uuid().to_string()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(records(&observer), (1, 0, 0));
    let proof = db.create_bound_conversation(&command).unwrap();
    assert_eq!(proof.sql_fence.position, 1);
    assert_eq!(
        observer
            .query_row(
                "select durable_count from agent_conversations where id=?1",
                [other.as_uuid().to_string()],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(metadata(&observer, command.id).2, 0);
    assert_eq!(
        observer
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn an_existing_unbound_or_differently_bound_conversation_is_never_adopted() {
    let (_dir, db, command) = fixture();
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title) values(?1,'Existing')",
            [command.id.as_uuid().to_string()],
        )
        .unwrap();
    assert!(matches!(
        db.create_bound_conversation(&command),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(records(&db.raw.lock().unwrap()), (1, 0, 0));
    db.lock().unwrap().execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','two')", [command.id.as_uuid().to_string()]).unwrap();
    assert!(matches!(
        db.create_bound_conversation(&command),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(records(&db.raw.lock().unwrap()), (1, 1, 0));
}

#[test]
fn original_proof_survives_deletion_without_recreating_or_authorizing_a_fresh_identity() {
    let (_dir, db, command) = fixture();
    let proof = db.create_bound_conversation(&command).unwrap();
    {
        let sql = db.lock().unwrap();
        sql.execute("insert into agent_conversation_deletions(request_id,conversation_id,request_hash) values(?1,?2,?3)",
            rusqlite::params![Uuid::new_v4().to_string(),command.id.as_uuid().to_string(),"a".repeat(64)]).unwrap();
        sql.execute(
            "delete from agent_conversations where id=?1",
            [command.id.as_uuid().to_string()],
        )
        .unwrap();
    }
    assert_eq!(
        db.create_bound_conversation(&command).unwrap(),
        SqlImmediateReceipt {
            replayed: true,
            ..proof.clone()
        }
    );
    assert!(
        matches!(db.bound_conversation_creation_status(&command).unwrap(), SqlImmediateStatus::Committed(value) if value.fingerprint==proof.fingerprint)
    );
    assert!(
        db.create_bound_conversation(&CreateBoundConversation {
            operation_id: Uuid::new_v4(),
            ..command
        })
        .is_err()
    );
    assert_eq!(records(&db.raw.lock().unwrap()), (0, 0, 1));
}

#[test]
fn write_rechecks_target_liveness_but_status_retains_the_original_historical_proof() {
    let (_dir, db, command) = fixture();
    let proof = db.create_bound_conversation(&command).unwrap();
    db.lock()
        .unwrap()
        .execute("delete from note_pages where id='one'", [])
        .unwrap();
    assert!(matches!(
        db.create_bound_conversation(&command),
        Err(SqlImmediateFailure::Domain(CatalogError::NotFound))
    ));
    assert!(
        matches!(db.bound_conversation_creation_status(&command).unwrap(), SqlImmediateStatus::Committed(value) if value.fingerprint==proof.fingerprint)
    );
    assert_eq!(records(&db.raw.lock().unwrap()), (1, 1, 1));
}

#[test]
fn changed_intent_and_cross_family_identity_conflict_without_mutation() {
    let (_dir, db, command) = fixture();
    db.create_bound_conversation(&command).unwrap();
    for changed in [
        CreateBoundConversation {
            id: ConversationId::from_uuid(Uuid::new_v4()),
            ..command.clone()
        },
        CreateBoundConversation {
            binding: ConversationBinding {
                resource_id: "two".into(),
                ..command.binding.clone()
            },
            ..command.clone()
        },
    ] {
        assert!(matches!(
            db.create_bound_conversation(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert!(matches!(
            db.bound_conversation_creation_status(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
    }
    let rename = RenameConversation {
        operation_id: command.operation_id,
        id: command.id,
        expected_title: "新对话".into(),
        title: "Other".into(),
    };
    assert!(matches!(
        db.rename_conversation(&rename),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        db.rename_status(&rename),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    let auth = super::super::AuthorizeRollback {
        operation_id: command.operation_id,
        conversation_id: command.id,
        binding: command.binding.clone(),
    };
    assert!(matches!(
        db.authorize_rollback(&auth),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        db.rollback_authorization_status(&auth),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(records(&db.raw.lock().unwrap()), (1, 1, 1));
    assert_eq!(metadata(&db.raw.lock().unwrap(), command.id).0, "新对话");
}

#[test]
fn journal_derived_identity_is_global_but_raw_batch_identity_is_not() {
    let (_dir, db, command) = fixture();
    let other = other_conversation(&db);
    db.write(
        pending(other, command.operation_id),
        WriteOptions::default(),
    )
    .unwrap();
    db.create_bound_conversation(&command).unwrap();
    let collision = CreateBoundConversation {
        operation_id: Uuid::new_v5(other.as_uuid(), command.operation_id.as_bytes()),
        ..command
    };
    assert!(matches!(
        db.create_bound_conversation(&collision),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(records(&db.raw.lock().unwrap()), (2, 1, 1));
}

#[test]
fn one_admission_covers_both_business_rows_and_proof_before_the_next_journal_writer() {
    let (dir, db, command) = fixture();
    let other = other_conversation(&db);
    let (entered, waiting) = mpsc::channel();
    let (proceed, permit) = mpsc::channel();
    let owned = db.clone();
    let intent = command.clone();
    let creation = std::thread::spawn(move || {
        owned.create_bound_conversation_owned(
            &intent,
            Fault::Pause {
                entered,
                proceed: permit,
            },
        )
    });
    waiting
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let observer = Connection::open(dir.path().join("create-bound.sqlite3")).unwrap();
    assert_eq!(records(&observer), (1, 0, 0));
    let (done, finished) = mpsc::channel();
    let writer = db.clone();
    let journal = std::thread::spawn(move || {
        done.send(writer.write(pending(other, Uuid::new_v4()), WriteOptions::default()))
            .unwrap()
    });
    assert!(
        finished
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
    proceed.send(()).unwrap();
    assert_eq!(creation.join().unwrap().unwrap().sql_fence.position, 0);
    finished
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    journal.join().unwrap();
    assert_eq!(records(&observer), (2, 1, 1));
    assert_eq!(metadata(&observer, command.id).2, 0);
}
