use super::super::Fault;
use super::*;
use crate::{agent::repository::SqliteConversationCatalog, data_interface::SharedDatabase};
use research_agent_core::conversation::{ConversationDeletionStore, ConversationId};
use rusqlite::{Connection, params};
use std::path::Path;
fn open(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("delete.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("log")).unwrap()
}
async fn fixture() -> (tempfile::TempDir, SharedDatabase, PreparedDeletion) {
    let root = tempfile::tempdir().unwrap();
    let db = open(root.path());
    let request = ConversationDeletionRequest {
        request_id: Uuid::new_v4(),
        conversation_id: ConversationId::from_uuid(Uuid::new_v4()),
        binding: None,
        expected_title: "Synthetic delete".into(),
        expected_durable_count: 0,
        force: false,
    };
    db.raw
        .lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title,archived) values(?1,?2,1)",
            params![
                request.conversation_id.as_uuid().to_string(),
                request.expected_title
            ],
        )
        .unwrap();
    let prepared = SqliteConversationCatalog::new(db.clone())
        .prepare_deletion(request)
        .await
        .unwrap();
    (root, db, prepared)
}

async fn second_prepared(db: &SharedDatabase) -> PreparedDeletion {
    let request = ConversationDeletionRequest {
        request_id: Uuid::new_v4(),
        conversation_id: ConversationId::from_uuid(Uuid::new_v4()),
        binding: None,
        expected_title: "Second synthetic delete".into(),
        expected_durable_count: 0,
        force: true,
    };
    db.raw
        .lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title,archived) values(?1,?2,1)",
            params![
                request.conversation_id.as_uuid().to_string(),
                request.expected_title
            ],
        )
        .unwrap();
    SqliteConversationCatalog::new(db.clone())
        .prepare_batch_deletion(request)
        .await
        .unwrap()
}

#[tokio::test]
async fn batch_deletion_is_atomic_and_all_receipts_survive_reopen() {
    let (root, db, first) = fixture().await;
    let second = second_prepared(&db).await;
    let requests = [first.request().clone(), second.request().clone()];
    db.delete_conversations_batch(&[first, second]).unwrap();
    assert_eq!(counts(&db), (0, 2, 2, 0));
    drop(db);
    let reopened = open(root.path());
    for request in requests {
        assert!(matches!(
            reopened.deletion_status(&request).unwrap(),
            SqlImmediateStatus::Committed(_)
        ));
    }
}

#[tokio::test]
async fn batch_deletion_rolls_back_every_member_when_one_confirmation_changes() {
    let (_root, db, first) = fixture().await;
    let second = second_prepared(&db).await;
    db.raw
        .lock()
        .unwrap()
        .execute(
            "update agent_conversations set title='changed' where id=?1",
            [second.request().conversation_id.as_uuid().to_string()],
        )
        .unwrap();
    assert!(matches!(
        db.delete_conversations_batch(&[first, second]),
        Err(DeletionFailure::Domain(_))
    ));
    assert_eq!(counts(&db), (2, 0, 0, 0));
}
fn counts(db: &SharedDatabase) -> (i64, i64, i64, i64) {
    let sql = db.raw.lock().unwrap();
    let count = |table| {
        sql.query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
            .unwrap()
    };
    (
        count("agent_conversations"),
        count("agent_conversation_deletions"),
        count("data_sql_operation_receipts"),
        count("agent_force_delete_markers"),
    )
}
#[tokio::test]
async fn managed_deletion_and_tombstone_are_atomic_exact_and_survive_reopen() {
    let (root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    let catalog = SqliteConversationCatalog::new(db.clone());
    let original = catalog.delete(request.clone()).await.unwrap();
    assert_eq!(counts(&db), (0, 1, 1, 0));
    let proof = db.deletion_status(&request).unwrap();
    assert!(matches!(proof, SqlImmediateStatus::Committed(_)));
    let changes = db.raw.lock().unwrap().total_changes();
    assert_eq!(catalog.delete(request.clone()).await.unwrap(), original);
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    drop(catalog);
    drop(db);
    let reopened = open(root.path());
    assert_eq!(reopened.deletion_status(&request).unwrap(), proof);
    assert_eq!(
        SqliteConversationCatalog::new(reopened.clone())
            .delete(request.clone())
            .await
            .unwrap(),
        original
    );
    assert!(
        reopened
            .raw
            .lock()
            .unwrap()
            .execute(
                "insert into agent_conversations(id,title) values(?1,'stale resurrection')",
                [request.conversation_id.as_uuid().to_string()]
            )
            .is_err()
    );
}
#[tokio::test]
async fn lost_commit_ack_and_unconfirmed_cleanup_preserve_the_original_identity() {
    for fault in [Fault::BeforeCommit, Fault::AfterCommit, Fault::Rollback] {
        let (root, db, prepared) = fixture().await;
        let request = prepared.request().clone();
        let after = matches!(&fault, Fault::AfterCommit);
        assert!(matches!(
            db.delete_conversation_owned(&prepared, fault),
            Err(DeletionFailure::Storage(SqlImmediateFailure::Unknown(_)))
        ));
        let changes = db.raw.lock().unwrap().total_changes();
        let status = db.deletion_status(&request).unwrap();
        assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
        assert!(if after {
            matches!(status, SqlImmediateStatus::Committed(_))
        } else {
            matches!(status, SqlImmediateStatus::Unconfirmed { .. })
        });
        if after {
            assert!(db.deletion_replay(&request).unwrap());
        } else {
            assert!(db.deletion_replay(&request).is_err());
        }
        drop(db);
        let reopened = open(root.path());
        assert!(if after {
            matches!(
                reopened.deletion_status(&request).unwrap(),
                SqlImmediateStatus::Committed(_)
            )
        } else {
            matches!(
                reopened.deletion_status(&request).unwrap(),
                SqlImmediateStatus::NoSqlReceipt { .. }
            )
        });
        SqliteConversationCatalog::new(reopened.clone())
            .delete(request)
            .await
            .unwrap();
        assert_eq!(counts(&reopened), (0, 1, 1, 0));
    }
}
#[tokio::test]
async fn receipt_and_cascade_failures_roll_back_and_retain_safe_sqlite_codes() {
    for (table, action) in [
        ("agent_conversations", "delete"),
        ("agent_conversation_deletions", "insert"),
        ("data_sql_operation_receipts", "insert"),
    ] {
        let (_root, db, prepared) = fixture().await;
        db.raw.lock().unwrap().execute_batch(&format!("create temp trigger refuse_delete before {action} on main.{table} begin select raise(abort,'private deletion trigger text');end;")).unwrap();
        let Err(DeletionFailure::Storage(cause)) = db.delete_conversation(&prepared) else {
            panic!("must retain storage failure");
        };
        let wire = serde_json::to_value(cause).unwrap();
        assert_eq!(wire["code"], "DATA_SQL_UNAVAILABLE");
        assert_eq!(wire["reason"]["sqliteExtendedCode"], 1811);
        assert!(!wire.to_string().contains("private deletion"));
        assert_eq!(counts(&db), (1, 0, 0, 0));
        assert!(db.raw.lock().unwrap().is_autocommit());
    }
}
#[tokio::test]
async fn changed_intent_and_cross_family_requests_do_not_mutate_or_reuse_proof() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.delete_conversation(&prepared).unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    for changed in [
        ConversationDeletionRequest {
            force: true,
            ..request.clone()
        },
        ConversationDeletionRequest {
            expected_title: "changed".into(),
            ..request.clone()
        },
        ConversationDeletionRequest {
            conversation_id: ConversationId::from_uuid(Uuid::new_v4()),
            ..request.clone()
        },
    ] {
        assert!(matches!(
            db.deletion_status(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert!(db.deletion_replay(&changed).is_err());
    }
    let rename = research_agent_core::conversation::RenameConversation {
        operation_id: request.request_id,
        id: request.conversation_id,
        expected_title: request.expected_title.clone(),
        title: "must not rename".into(),
    };
    assert!(matches!(
        db.rename_conversation(&rename),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
}
#[tokio::test]
async fn legacy_tombstones_replay_without_backfilling_managed_proof() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.delete_conversation(&prepared).unwrap();
    db.raw
        .lock()
        .unwrap()
        .execute(
            "delete from data_sql_operation_receipts where operation_id=?1",
            [request.request_id.to_string()],
        )
        .unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    assert!(db.deletion_replay(&request).unwrap());
    db.delete_conversation(&prepared).unwrap();
    SqliteConversationCatalog::new(db.clone())
        .delete(request.clone())
        .await
        .unwrap();
    assert_eq!(counts(&db), (0, 1, 0, 0));
    assert!(matches!(
        db.deletion_status(&request).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
}
#[tokio::test]
async fn a_domain_refusal_cannot_hide_a_failed_rollback() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.raw
        .lock()
        .unwrap()
        .execute("update agent_conversations set archived=0", [])
        .unwrap();
    assert!(matches!(
        db.delete_conversation(&prepared),
        Err(DeletionFailure::Domain(
            ConversationDeletionError::NotArchived
        ))
    ));
    assert!(matches!(
        db.delete_conversation_owned(&prepared, Fault::Rollback),
        Err(DeletionFailure::Storage(SqlImmediateFailure::Unknown(_)))
    ));
    assert!(matches!(
        db.deletion_status(&request).unwrap(),
        SqlImmediateStatus::Unconfirmed { .. }
    ));
}
#[tokio::test]
async fn new_workspace_after_cleanup_preparation_is_never_silently_purged() {
    let (_root, db, prepared) = fixture().await;
    let mut request = prepared.request().clone();
    request.force = true;
    let prepared = SqliteConversationCatalog::new(db.clone())
        .prepare_deletion(request)
        .await
        .unwrap();
    let id = prepared.request().conversation_id.as_uuid().to_string();
    db.raw.lock().unwrap().execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now');insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now');").unwrap();
    db.raw.lock().unwrap().execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,'new-request','new-turn','new-context',1,'{}')",[&id]).unwrap();
    db.raw.lock().unwrap().execute("insert into agent_execution_workspaces(conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json) values(?1,'new-request','new-turn','new-context','m','new-lease',?2,'{}')",params![id,"a".repeat(64)]).unwrap();
    assert!(matches!(
        db.delete_conversation(&prepared),
        Err(DeletionFailure::Domain(ConversationDeletionError::Catalog(
            CatalogError::Conflict
        )))
    ));
    assert_eq!(counts(&db), (1, 0, 0, 0));
}
#[tokio::test]
async fn unmanaged_catalog_never_falls_back_to_direct_sql() {
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let raw = std::sync::Arc::new(std::sync::Mutex::new(sql));
    let catalog = SqliteConversationCatalog::new(raw.clone());
    let (_root, _db, prepared) = fixture().await;
    let changes = raw.lock().unwrap().total_changes();
    assert!(matches!(
        catalog.delete(prepared.request().clone()).await,
        Err(ConversationDeletionError::Data(
            research_agent_core::conversation::CatalogDataFailure {
                kind: research_agent_core::conversation::CatalogDataFailureKind::Unmanaged,
                ..
            }
        ))
    ));
    assert_eq!(raw.lock().unwrap().total_changes(), changes);
}

#[tokio::test]
async fn status_never_drains_the_log_but_deletion_publishes_after_the_accepted_suffix() {
    use crate::data_interface::journal::JournalCommand;
    use research_agent_core::protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind};
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    let other = ConversationId::from_uuid(Uuid::new_v4());
    db.raw
        .lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title) values(?1,'Other stream')",
            [other.as_uuid().to_string()],
        )
        .unwrap();
    let batch = Uuid::new_v4();
    let event = AgentEvent::new(
        other,
        1,
        1,
        batch,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "synthetic".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
    )
    .unwrap();
    db.write(
        JournalCommand::new(other, batch, 0, vec![event], None, 1).unwrap(),
        research_data_runtime::WriteOptions::default(),
    )
    .unwrap();
    let before = db.benchmark_snapshot().unwrap();
    assert!(before.accepted_position > before.materialized_position);
    let changes = db.raw.lock().unwrap().total_changes();
    assert!(matches!(
        db.deletion_status(&request).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert_eq!(db.benchmark_snapshot().unwrap(), before);
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    db.delete_conversation(&prepared).unwrap();
    let SqlImmediateStatus::Committed(proof) = db.deletion_status(&request).unwrap() else {
        panic!("expected original proof");
    };
    assert_eq!(proof.sql_fence.position, before.accepted_position);
    let materialized: u64 = db
        .raw
        .lock()
        .unwrap()
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [other.as_uuid().to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(materialized, 1);
}
#[tokio::test]
async fn real_deferred_constraint_commit_failure_blocks_until_reopen() {
    let (root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.raw.lock().unwrap().execute_batch("create temp table delete_commit_parent(id integer primary key);create temp table delete_commit_child(parent integer references delete_commit_parent(id) deferrable initially deferred);create temp trigger reject_delete_commit after insert on main.data_sql_operation_receipts when new.kind='agent.conversation.delete' begin insert into delete_commit_child values(7);end;").unwrap();
    let Err(DeletionFailure::Storage(cause)) = db.delete_conversation(&prepared) else {
        panic!("expected failure");
    };
    let wire = serde_json::to_value(cause).unwrap();
    assert_eq!(wire["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(wire["reason"]["sqliteExtendedCode"], 787);
    assert_eq!(counts(&db), (1, 0, 0, 0));
    assert!(matches!(
        db.deletion_status(&request).unwrap(),
        SqlImmediateStatus::Unconfirmed { .. }
    ));
    drop(db);
    let reopened = open(root.path());
    assert!(matches!(
        reopened.deletion_status(&request).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert_eq!(counts(&reopened), (1, 0, 0, 0));
}

#[tokio::test]
async fn a_closed_owner_can_return_an_existing_receipt_without_granting_a_new_deletion() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.delete_conversation(&prepared).unwrap();
    db.shutdown().unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    assert!(db.deletion_replay(&request).unwrap());
    SqliteConversationCatalog::new(db.clone())
        .delete(request.clone())
        .await
        .unwrap();
    assert!(matches!(
        db.deletion_status(&request).unwrap(),
        SqlImmediateStatus::Committed(_)
    ));
    let changed = ConversationDeletionRequest {
        request_id: Uuid::new_v4(),
        ..request
    };
    assert!(db.deletion_replay(&changed).is_err());
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
}

#[tokio::test]
async fn force_metadata_cannot_be_deleted_without_the_original_workspace_file_owner() {
    let (_root, db, prepared) = fixture().await;
    let mut request = prepared.request().clone();
    request.force = true;
    let id = request.conversation_id.as_uuid().to_string();
    db.raw.lock().unwrap().execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now');insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now');").unwrap();
    db.raw.lock().unwrap().execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,'request','turn','context',1,'{}')",[&id]).unwrap();
    db.raw.lock().unwrap().execute("insert into agent_execution_workspaces(conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json) values(?1,'request','turn','context','m','lease',?2,'{}')",params![id,"a".repeat(64)]).unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    let Err(ConversationDeletionError::Data(cause)) = SqliteConversationCatalog::new(db.clone())
        .delete(request)
        .await
    else {
        panic!("missing file owner must refuse metadata-only cleanup");
    };
    assert_eq!(
        cause.diagnostic.safe_cause,
        "workspace_directory_unavailable"
    );
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    assert_eq!(counts(&db), (1, 0, 0, 0));
}

#[path = "tests/epoch.rs"]
mod epoch;
