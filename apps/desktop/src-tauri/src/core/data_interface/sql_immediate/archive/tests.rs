use super::super::Fault;
use super::*;
use crate::{agent::repository::SqliteConversationCatalog, data_interface::SharedDatabase};
use research_agent_core::conversation::{CatalogError, ConversationCatalog, ConversationId};
use rusqlite::Connection;
use std::path::Path;

#[path = "tests/failures.rs"]
mod failures;

fn open(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("archive.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("data-log")).unwrap()
}
async fn fixture() -> (
    tempfile::TempDir,
    SharedDatabase,
    ArchiveConversation,
    ValidatedArchive,
) {
    let root = tempfile::tempdir().unwrap();
    let db = open(root.path());
    let command = ArchiveConversation {
        operation_id: Uuid::new_v4(),
        id: ConversationId::from_uuid(Uuid::new_v4()),
        archived: true,
        expected_watermark: Some(0),
    };
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title) values(?1,'Archive fixture')",
            [command.id.as_uuid().to_string()],
        )
        .unwrap();
    let prepared = SqliteConversationCatalog::new(db.clone())
        .validate_archive(command.clone())
        .await
        .unwrap();
    (root, db, command, prepared)
}
fn metadata(db: &Connection, command: &ArchiveConversation) -> (bool, String) {
    db.query_row(
        "select archived,updated_at from agent_conversations where id=?1",
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

#[tokio::test]
async fn w0_archive_restore_old_archive_replays_without_current_metadata_or_history() {
    let (_root, db, command, _) = fixture().await;
    let catalog = SqliteConversationCatalog::new(db.clone());
    let mut proof = catalog.set_archived(command.clone()).await.unwrap();
    assert_eq!(proof.sql_fence, 0);
    assert!(!proof.replayed);
    let restore = ArchiveConversation {
        operation_id: Uuid::new_v4(),
        archived: false,
        expected_watermark: None,
        ..command.clone()
    };
    catalog.set_archived(restore).await.unwrap();
    let before = metadata(&db.lock().unwrap(), &command);
    // Fresh export would reject this history. An exact original proof must win.
    db.lock()
        .unwrap()
        .execute(
            "update agent_conversations set durable_count=1 where id=?1",
            [command.id.as_uuid().to_string()],
        )
        .unwrap();
    db.lock().unwrap().execute_batch("create trigger deny_archive_replay before update of archived on agent_conversations begin select raise(abort,'unexpected replay'); end;").unwrap();
    proof.replayed = true;
    assert_eq!(catalog.set_archived(command.clone()).await.unwrap(), proof);
    assert_eq!(metadata(&db.lock().unwrap(), &command), before);
    let sql = db.lock().unwrap();
    assert_eq!(receipt_count(&sql), 2);
    assert_eq!(sql.query_row("select kind,version,result_json from data_sql_operation_receipts where operation_id=?1",[command.operation_id.to_string()],|row|Ok((row.get::<_,String>(0)?,row.get::<_,u32>(1)?,row.get::<_,String>(2)?))).unwrap(),(KIND.into(),VERSION,"null".into()));
    drop(sql);
    db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversations where id=?1",
            [command.id.as_uuid().to_string()],
        )
        .unwrap();
    assert_eq!(catalog.set_archived(command).await.unwrap(), proof);
    assert_eq!(
        db.lock()
            .unwrap()
            .query_row("select count(*) from agent_conversations", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn changed_intent_cross_family_and_post_validation_watermark_are_rejected_atomically() {
    let (_root, db, command, prepared) = fixture().await;
    db.lock()
        .unwrap()
        .execute("update agent_conversations set durable_count=1", [])
        .unwrap();
    assert!(matches!(
        db.archive_conversation(&prepared),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(receipt_count(&db.lock().unwrap()), 0);
    db.lock()
        .unwrap()
        .execute("update agent_conversations set durable_count=0", [])
        .unwrap();
    db.archive_conversation(&prepared).unwrap();
    let changed = ArchiveConversation {
        archived: false,
        expected_watermark: None,
        ..command.clone()
    };
    assert!(matches!(
        db.archive_replay(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        db.archive_status(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    let rename = research_agent_core::conversation::RenameConversation {
        operation_id: command.operation_id,
        id: command.id,
        expected_title: "Archive fixture".into(),
        title: "Other".into(),
    };
    assert!(matches!(
        db.rename_conversation(&rename),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(metadata(&db.lock().unwrap(), &command).0);
    assert_eq!(receipt_count(&db.lock().unwrap()), 1);
}

#[tokio::test]
async fn closed_owner_inspects_proof_but_rejects_replay_write() {
    let (_root, db, command, prepared) = fixture().await;
    db.archive_conversation(&prepared).unwrap();
    db.shutdown().unwrap();
    assert!(matches!(
        db.archive_status(&command).unwrap(),
        SqlImmediateStatus::Committed(_)
    ));
    assert!(db.archive_replay(&command).is_err());
    assert!(db.archive_conversation(&prepared).is_err());
}

#[tokio::test]
async fn status_does_not_drain_pending_history_but_archive_validates_the_materialized_prefix() {
    use crate::data_interface::journal::JournalCommand;
    use research_agent_core::protocol::{
        AgentEvent, EventPayload, ModelIdentity, TaskKind, TurnOutcome,
    };
    use research_data_runtime::WriteOptions;
    let (root, db, mut command, _) = fixture().await;
    let turn = Uuid::new_v4();
    let payloads = [
        EventPayload::TurnStarted {
            turn_id: turn,
            task_kind: TaskKind::Regular,
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "test".into(),
                revision: None,
            },
        },
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        },
    ];
    let events = payloads
        .into_iter()
        .enumerate()
        .map(|(n, payload)| {
            AgentEvent::new(command.id, n as u64 + 1, 0, Uuid::new_v4(), payload).unwrap()
        })
        .collect();
    db.write(
        JournalCommand::new(
            command.id,
            Uuid::new_v4(),
            0,
            events,
            None,
            1_790_000_000_000,
        )
        .unwrap(),
        WriteOptions::default(),
    )
    .unwrap();
    command.expected_watermark = Some(2);
    let observer = Connection::open(root.path().join("archive.sqlite3")).unwrap();
    let durable = || {
        observer
            .query_row("select durable_count from agent_conversations", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap()
    };
    assert_eq!(durable(), 0);
    assert!(matches!(
        db.archive_status(&command).unwrap(),
        SqlImmediateStatus::NoSqlReceipt {
            sql_fence: ReadFence { position: 0, .. },
            ..
        }
    ));
    assert_eq!(durable(), 0);
    let catalog = SqliteConversationCatalog::new(db.clone());
    let proof = catalog.set_archived(command.clone()).await.unwrap();
    assert_eq!(durable(), 2);
    assert_eq!(proof.sql_fence, 1);
    assert!(metadata(&observer, &command).0);
}
