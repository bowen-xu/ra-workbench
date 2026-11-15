use super::*;
use crate::{
    agent::repository::{SqliteConversationCatalog, journal as journal_storage},
    data_interface::{AuthorizeRollback, Database, SqlImmediateStatus, journal::JournalCommand},
};
use research_agent_core::{
    conversation::{
        BindingKind, ConversationBinding, ConversationCatalog, ConversationId,
        CreateBoundConversation, CreateBoundStatus, HistoryQuery, RenameConversation, RenameProof,
        RenameStatus,
    },
    protocol::{
        AgentEvent, ConversationProjection, EventPayload, ItemKind, ModelIdentity, TaskKind,
        TurnOutcome,
    },
};
use research_data_runtime::{RaftLog, WriteOptions};
use rusqlite::params;
use uuid::Uuid;

type ReceiptRow = (String, String, String, u32, Vec<u8>, u64, String);

fn receipt_rows(sql: &Connection) -> Vec<ReceiptRow> {
    sql.prepare(
        "select operation_id,epoch,kind,version,fingerprint,sql_fence_position,result_json
         from data_sql_operation_receipts order by operation_id",
    )
    .unwrap()
    .query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

fn pending_events(id: ConversationId) -> Vec<AgentEvent> {
    let request = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let payloads = [
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "backup".into(),
                revision: None,
            },
        },
        EventPayload::ItemAccepted {
            turn_id: turn,
            item_id: Uuid::new_v4(),
            kind: ItemKind::AssistantMessage,
            text: "备份中的原始事实\0🙂".into(),
            phase: None,
            source: None,
        },
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        },
    ];
    let mut projection = ConversationProjection::new(id);
    payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            let event = AgentEvent::new(id, index as u64 + 1, 1, request, payload).unwrap();
            projection.apply(&event).unwrap();
            event
        })
        .collect()
}

fn replayed(proof: &RenameProof) -> RenameProof {
    RenameProof {
        replayed: true,
        ..proof.clone()
    }
}

#[tokio::test]
async fn rename_proofs_and_legacy_unit_receipt_survive_paired_backup_with_pending_history() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("research-assistant.sqlite3");
    let log_path = root.path().join("data-log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 213);
    assert_eq!(
        sql.query_row("select max(version) from schema_migrations", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        213
    );
    let database = Database::open(sql, &log_path).unwrap();
    let catalog = SqliteConversationCatalog::new(database.clone());
    let id = ConversationId::from_uuid(Uuid::new_v4());
    let note = Uuid::new_v4().to_string();
    let authorization = AuthorizeRollback {
        operation_id: Uuid::new_v4(),
        conversation_id: id,
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: note.clone(),
        },
    };
    {
        let sql = database.lock().unwrap();
        sql.execute(
            "insert into note_pages(id,title,created_at,updated_at) values(?1,'backup note','now','now')",
            [&note],
        )
        .unwrap();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'A')",
            [id.as_uuid().to_string()],
        )
        .unwrap();
        sql.execute(
            "insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note',?2)",
            params![id.as_uuid().to_string(), note],
        )
        .unwrap();
    }
    let legacy = database.authorize_rollback(&authorization).unwrap();
    let first = RenameConversation {
        operation_id: Uuid::new_v4(),
        id,
        expected_title: "A".into(),
        title: " B ".into(),
    };
    let second = RenameConversation {
        operation_id: Uuid::new_v4(),
        id,
        expected_title: "B".into(),
        title: "A".into(),
    };
    let first_proof = catalog.rename(first.clone()).await.unwrap();
    let second_proof = catalog.rename(second.clone()).await.unwrap();
    assert_eq!((first_proof.sql_fence, second_proof.sql_fence), (0, 0));
    assert_eq!(first_proof.epoch, legacy.epoch);
    let creation = CreateBoundConversation {
        operation_id: Uuid::new_v4(),
        id: ConversationId::from_uuid(Uuid::new_v4()),
        binding: authorization.binding.clone(),
    };
    let creation_proof = catalog.create_bound(creation.clone()).await.unwrap();
    assert_eq!(
        (creation_proof.epoch, creation_proof.sql_fence),
        (legacy.epoch, 0)
    );
    let creation_metadata = catalog
        .read(creation.id, Some(creation.binding.clone()))
        .await
        .unwrap();
    let expected_rows = receipt_rows(&database.lock().unwrap());
    assert_eq!(expected_rows.len(), 4);
    for row in &expected_rows {
        assert_eq!(row.1, legacy.epoch.to_string());
        assert_eq!(row.3, 1);
        assert_eq!(row.4.len(), 32);
        assert_eq!((row.5, row.6.as_str()), (0, "null"));
        assert_eq!(
            row.2,
            if row.0 == authorization.operation_id.to_string() {
                "agent.rollback_authority"
            } else if row.0 == creation.operation_id.to_string() {
                "agent.conversation.create_bound"
            } else {
                "agent.conversation.rename"
            }
        );
    }
    let events = pending_events(id);
    let accepted = database
        .write(
            JournalCommand::new(
                id,
                Uuid::new_v4(),
                0,
                events.clone(),
                None,
                1_790_000_000_000,
            )
            .unwrap(),
            WriteOptions::default(),
        )
        .unwrap();
    assert_eq!(accepted.position, 1);
    assert_eq!(accepted.epoch, legacy.epoch);
    assert_eq!(
        catalog.rename_status(first.clone()).await.unwrap(),
        RenameStatus::Committed(replayed(&first_proof))
    );
    assert_eq!(
        catalog.create_bound_status(creation.clone()).await.unwrap(),
        CreateBoundStatus::Committed(replayed(&creation_proof))
    );
    // No shutdown or compatibility query: retain the accepted, unprojected suffix.
    drop(catalog);
    drop(database);
    let source = Connection::open(&path).unwrap();
    assert_eq!(receipt_rows(&source), expected_rows);
    let prefix: (u64, u64, String) = source
        .query_row(
            "select materialized_position,durable_count,title from data_runtime_state,agent_conversations where id=?1",
            [id.as_uuid().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(prefix, (0, 0, "A".into()));
    let locked_source = Database::lock_existing_pair(&source, &log_path)
        .unwrap()
        .unwrap();
    assert_eq!(locked_source.head().unwrap().accepted_position, 1);
    // 214 selects the actual backup path only. No new migration is registered or run.
    let backup = backup_before_version(
        &source,
        &root.path().join("backups"),
        Some(&locked_source),
        214,
    )
    .unwrap()
    .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(backup.with_extension("pair.json")).unwrap()).unwrap();
    assert_eq!(manifest["complete"], true);
    assert_eq!(manifest["epoch"], legacy.epoch.to_string());
    assert_eq!(manifest["materializedPosition"], 0);
    assert_eq!(manifest["acceptedPosition"], 1);
    let backup_sql = Connection::open(&backup).unwrap();
    assert_eq!(receipt_rows(&backup_sql), expected_rows);
    let backup_log = backup.with_extension("raft");
    let locked = Database::lock_existing_pair(&backup_sql, &backup_log)
        .unwrap()
        .unwrap();
    let (sql, locked) =
        Database::prepare_schema_migration(backup_sql, &backup_log, locked).unwrap();
    let restored = Database::open_with_log(sql, &backup_log, Some(locked)).unwrap();
    let restored_catalog = SqliteConversationCatalog::new(restored.clone());
    assert_eq!(
        restored_catalog
            .create_bound_status(creation.clone())
            .await
            .unwrap(),
        CreateBoundStatus::Committed(replayed(&creation_proof))
    );
    assert_eq!(
        restored_catalog
            .create_bound(creation.clone())
            .await
            .unwrap(),
        replayed(&creation_proof)
    );
    assert_eq!(
        restored_catalog
            .read(creation.id, Some(creation.binding))
            .await
            .unwrap(),
        creation_metadata
    );
    assert_eq!(
        restored_catalog.rename_status(first.clone()).await.unwrap(),
        RenameStatus::Committed(replayed(&first_proof))
    );
    assert_eq!(
        restored_catalog.rename(first).await.unwrap(),
        replayed(&first_proof)
    );
    assert_eq!(
        restored_catalog.rename(second).await.unwrap(),
        replayed(&second_proof)
    );
    assert_eq!(restored_catalog.read(id, None).await.unwrap().title, "A");
    let original_authority = match restored
        .rollback_authorization_status(&authorization)
        .unwrap()
    {
        SqlImmediateStatus::Committed(proof) => proof,
        other => panic!("expected original legacy unit receipt, got {other:?}"),
    };
    assert_eq!(original_authority.epoch, legacy.epoch);
    assert_eq!(original_authority.sql_fence, legacy.sql_fence);
    assert_eq!(original_authority.fingerprint, legacy.fingerprint);
    assert_eq!(
        restored.authorize_rollback(&authorization).unwrap(),
        original_authority
    );
    {
        let mut sql = restored.lock().unwrap();
        assert_eq!(receipt_rows(&sql), expected_rows);
        assert_eq!(
            sql.query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sql.query_row("select count(*) from data_operation_receipts", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
            1
        );
        let tx = sql.transaction().unwrap();
        let history = journal_storage::read_history_in::<AgentEvent>(
            &tx,
            HistoryQuery::new(id, 0, 500).unwrap(),
        )
        .unwrap();
        assert_eq!(history.durable_count, events.len() as u64);
        assert_eq!(history.items, events);
        tx.commit().unwrap();
    }
    restored.shutdown().unwrap();
    drop(restored_catalog);
    drop(restored);
    let reopened = RaftLog::open(&backup_log, Some(legacy.epoch)).unwrap();
    assert_eq!(reopened.head().unwrap().accepted_position, 1);
    assert_eq!(receipt_rows(&source), expected_rows);
    assert!(Database::lock_existing_pair(&source, &log_path).is_err());
}
