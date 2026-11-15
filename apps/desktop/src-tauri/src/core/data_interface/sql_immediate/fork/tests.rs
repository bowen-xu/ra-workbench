use super::super::Fault;
use super::*;
use crate::{
    agent::repository::SqliteConversationCatalog,
    data_interface::{SharedDatabase, journal::JournalCommand},
};
use research_agent_core::{
    conversation::{
        ArchiveConversation, BindingKind, ConversationBinding, ConversationCatalog,
        ConversationForkStore, ConversationId, CreateBoundConversation, RenameConversation,
    },
    protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind, TurnOutcome},
};
use rusqlite::{Connection, params};
use std::path::Path;

fn open(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("fork.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("log")).unwrap()
}
async fn fixture() -> (tempfile::TempDir, SharedDatabase, PreparedFork) {
    let root = tempfile::tempdir().unwrap();
    let db = open(root.path());
    db.lock().unwrap().execute("insert into note_pages(id,title,created_at,updated_at) values('one','Note','now','now')",[]).unwrap();
    let parent = ConversationId::from_uuid(Uuid::new_v4());
    let child = ConversationId::from_uuid(Uuid::new_v4());
    let binding = ConversationBinding {
        kind: BindingKind::Note,
        resource_id: "one".into(),
    };
    db.create_bound_conversation(&CreateBoundConversation {
        operation_id: Uuid::new_v4(),
        id: parent,
        binding: binding.clone(),
    })
    .unwrap();
    let turn = Uuid::new_v4();
    let batch = Uuid::new_v4();
    let events = [
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "synthetic".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, payload)| AgentEvent::new(parent, index as u64 + 1, 1, batch, payload).unwrap())
    .collect::<Vec<_>>();
    db.write(
        JournalCommand::new(parent, batch, 0, events.clone(), None, 1).unwrap(),
        research_data_runtime::WriteOptions::default(),
    )
    .unwrap();
    let catalog = SqliteConversationCatalog::new(db.clone());
    let summary = catalog.read(parent, None).await.unwrap();
    let prepared = PreparedFork::prepare(
        ConversationForkRequest {
            child_id: child,
            parent_id: parent,
            through_turn_id: turn,
            binding,
        },
        summary,
        &events,
    )
    .unwrap();
    (root, db, prepared)
}
fn count(db: &SharedDatabase, table: &str, column: &str, child: ConversationId) -> i64 {
    db.raw
        .lock()
        .unwrap()
        .query_row(
            &format!("select count(*) from {table} where {column}=?1"),
            [child.as_uuid().to_string()],
            |row| row.get(0),
        )
        .unwrap()
}
fn assert_absent(db: &SharedDatabase, prepared: &PreparedFork) {
    let child = prepared.request().child_id;
    for (table, column) in [
        ("agent_conversations", "id"),
        ("agent_conversation_bindings", "conversation_id"),
        ("agent_items", "conversation_id"),
        ("agent_fork_turn_sources", "conversation_id"),
        ("agent_conversation_forks", "child_id"),
        ("agent_initial_context_windows", "conversation_id"),
        ("data_sql_operation_receipts", "operation_id"),
    ] {
        assert_eq!(count(db, table, column, child), 0, "{table}");
    }
}

#[tokio::test]
async fn legacy_lineage_is_readable_without_fabricating_a_managed_publication_proof() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.fork_conversation(&prepared).unwrap();
    // Pre-facade rows have lineage/history, but no data_sql_operation_receipts row.
    db.raw
        .lock()
        .unwrap()
        .execute(
            "delete from data_sql_operation_receipts where operation_id=?1",
            [request.child_id.as_uuid().to_string()],
        )
        .unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    let catalog = SqliteConversationCatalog::new(db.clone());
    assert_eq!(
        catalog.fork(request.clone()).await.unwrap().parent_end_seq,
        2
    );
    assert!(matches!(
        db.fork_status(&request).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
    assert!(matches!(
        db.fork_conversation(&prepared),
        Err(SqlImmediateFailure::Domain(
            research_agent_core::conversation::CatalogError::Conflict
        ))
    ));
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    assert!(catalog.fork_legacy_for_tests(request).await.is_err());
}

#[tokio::test]
async fn an_unmanaged_production_catalog_cannot_fall_back_to_sql_only_creation() {
    let (_root, _managed, prepared) = fixture().await;
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let raw = std::sync::Arc::new(std::sync::Mutex::new(sql));
    let catalog = SqliteConversationCatalog::new(raw.clone());
    let changes = raw.lock().unwrap().total_changes();
    let error = catalog.fork(prepared.request().clone()).await.unwrap_err();
    assert!(matches!(
        error,
        research_agent_core::conversation::CatalogMutationError::Data(
            research_agent_core::conversation::CatalogDataFailure {
                kind: research_agent_core::conversation::CatalogDataFailureKind::Unmanaged,
                ..
            }
        )
    ));
    assert_eq!(raw.lock().unwrap().total_changes(), changes);
}
#[tokio::test]
async fn managed_catalog_replay_keeps_current_metadata_and_the_original_publication_proof() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    let catalog = SqliteConversationCatalog::new(db.clone());
    let child = catalog.fork(request.clone()).await.unwrap();
    assert_eq!(child.parent_end_seq, 2);
    assert_eq!(child.conversation.durable_count, 2);
    let original = db.fork_status(&request).unwrap();
    catalog
        .rename(RenameConversation {
            operation_id: Uuid::new_v4(),
            id: request.child_id,
            expected_title: child.conversation.title,
            title: "Later child name".into(),
        })
        .await
        .unwrap();
    catalog
        .set_archived(ArchiveConversation {
            operation_id: Uuid::new_v4(),
            id: request.child_id,
            archived: true,
            expected_watermark: Some(2),
        })
        .await
        .unwrap();
    // A broken later parent snapshot must never trigger fresh preparation on replay.
    db.raw
        .lock()
        .unwrap()
        .execute(
            "update agent_conversations set durable_count=99 where id=?1",
            [request.parent_id.as_uuid().to_string()],
        )
        .unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    let replayed = catalog.fork(request.clone()).await.unwrap();
    assert_eq!(replayed.conversation.title, "Later child name");
    assert!(replayed.conversation.archived);
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    assert_eq!(db.fork_status(&request).unwrap(), original);
    assert_eq!(
        count(&db, "agent_items", "conversation_id", request.child_id),
        2
    );
    for table in ["agent_execution_workspaces", "agent_input_queues"] {
        assert_eq!(count(&db, table, "conversation_id", request.child_id), 0);
    }
}
#[tokio::test]
async fn committed_but_lost_ack_is_observable_while_blocked_and_reopens_without_duplicate_history()
{
    let (root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    let error = db
        .fork_conversation_owned(&prepared, Fault::AfterCommit)
        .unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Unknown(_)));
    let proof = db.fork_status(&request).unwrap();
    assert!(matches!(proof, SqlImmediateStatus::Committed(_)));
    assert!(db.lock().is_err());
    assert!(db.fork_replay(&request).is_err());
    drop(db);
    let reopened = open(root.path());
    assert_eq!(reopened.fork_status(&request).unwrap(), proof);
    let catalog = SqliteConversationCatalog::new(reopened.clone());
    let child = catalog.fork(request.clone()).await.unwrap();
    assert_eq!(child.parent_end_seq, 2);
    assert_eq!(
        count(
            &reopened,
            "agent_items",
            "conversation_id",
            request.child_id
        ),
        2
    );
    assert_eq!(
        count(
            &reopened,
            "agent_initial_context_windows",
            "conversation_id",
            request.child_id
        ),
        1
    );
}
#[tokio::test]
async fn before_commit_and_unconfirmed_rollback_do_not_claim_absence_until_reopen() {
    for fault in [Fault::BeforeCommit, Fault::Rollback] {
        let (root, db, prepared) = fixture().await;
        let request = prepared.request().clone();
        assert!(matches!(
            db.fork_conversation_owned(&prepared, fault),
            Err(SqlImmediateFailure::Unknown(_))
        ));
        assert!(matches!(
            db.fork_status(&request).unwrap(),
            SqlImmediateStatus::Unconfirmed { .. }
        ));
        drop(db);
        let reopened = open(root.path());
        assert!(matches!(
            reopened.fork_status(&request).unwrap(),
            SqlImmediateStatus::NoSqlReceipt { .. }
        ));
        assert_absent(&reopened, &prepared);
    }
}
#[tokio::test]
async fn real_publication_and_receipt_trigger_failures_roll_back_the_whole_child() {
    for table in [
        "agent_conversation_bindings",
        "agent_fork_turn_sources",
        "agent_initial_context_windows",
        "data_sql_operation_receipts",
    ] {
        let (_root, db, prepared) = fixture().await;
        db.raw.lock().unwrap().execute_batch(&format!("create temp trigger refuse_fork before insert on main.{table} begin select raise(abort,'private synthetic fork cause'); end;")).unwrap();
        let error = db.fork_conversation(&prepared).unwrap_err();
        let wire = serde_json::to_value(&error).unwrap();
        assert!(!wire.to_string().contains("private synthetic"));
        assert_absent(&db, &prepared);
        assert!(db.raw.lock().unwrap().is_autocommit());
        assert_eq!(wire["code"], "DATA_SQL_UNAVAILABLE");
        if table != "agent_initial_context_windows" {
            assert_eq!(wire["reason"]["sqliteExtendedCode"], 1811);
        }
    }
}

#[tokio::test]
async fn a_real_deferred_constraint_commit_failure_remains_unknown_and_reopens_without_partial_child()
 {
    let (root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.raw.lock().unwrap().execute_batch(
        "create temp table commit_parent(id integer primary key);
         create temp table commit_child(parent integer references commit_parent(id) deferrable initially deferred);
         create temp trigger reject_fork_commit after insert on main.data_sql_operation_receipts
         when new.kind='agent.conversation.fork'
         begin insert into commit_child(parent) values(7); end;"
    ).unwrap();
    let error = db.fork_conversation(&prepared).unwrap_err();
    let wire = serde_json::to_value(&error).unwrap();
    assert_eq!(wire["code"], "DATA_SQL_COMMIT_UNKNOWN");
    assert_eq!(wire["reason"]["phase"], "commit");
    assert_eq!(wire["reason"]["sqliteExtendedCode"], 787);
    assert_absent(&db, &prepared);
    assert!(matches!(
        db.fork_status(&request).unwrap(),
        SqlImmediateStatus::Unconfirmed { .. }
    ));
    drop(db);
    let reopened = open(root.path());
    assert_absent(&reopened, &prepared);
    assert!(matches!(
        reopened.fork_status(&request).unwrap(),
        SqlImmediateStatus::NoSqlReceipt { .. }
    ));
}
#[tokio::test]
async fn exact_intent_and_cross_kind_identity_are_checked_without_republication() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.fork_conversation(&prepared).unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    let mut changed = request.clone();
    changed.through_turn_id = Uuid::new_v4();
    assert!(matches!(
        db.fork_status(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        db.fork_replay(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    let collision = RenameConversation {
        operation_id: *request.child_id.as_uuid(),
        id: request.parent_id,
        expected_title: "New chat".into(),
        title: "Must not rename".into(),
    };
    assert!(matches!(
        db.rename_conversation(&collision),
        Err(SqlImmediateFailure::Conflict(_))
    ));
}
#[tokio::test]
async fn status_does_not_drain_pending_parent_events_or_read_mutable_business_rows() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.fork_conversation(&prepared).unwrap();
    let original = db.fork_status(&request).unwrap();
    let batch = Uuid::new_v4();
    let events = vec![
        AgentEvent::new(
            request.parent_id,
            3,
            2,
            batch,
            EventPayload::TurnStarted {
                task_kind: TaskKind::Regular,
                turn_id: Uuid::new_v4(),
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "synthetic".into(),
                    model_id: "later".into(),
                    revision: None,
                },
            },
        )
        .unwrap(),
    ];
    db.write(
        JournalCommand::new(request.parent_id, batch, 2, events, None, 2).unwrap(),
        research_data_runtime::WriteOptions::default(),
    )
    .unwrap();
    let changes = db.raw.lock().unwrap().total_changes();
    assert_eq!(db.fork_status(&request).unwrap(), original);
    let raw = db.raw.lock().unwrap();
    assert_eq!(raw.total_changes(), changes);
    let count: u64 = raw
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            params![request.parent_id.as_uuid().to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn fork_epoch_fence_rejects_before_projecting_a_foreign_owners_pending_suffix() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    let epoch = match db.fork_status(&request).unwrap() {
        SqlImmediateStatus::NoSqlReceipt { epoch, .. } => epoch,
        _ => panic!("fresh child has no proof"),
    };
    let batch = Uuid::new_v4();
    let event = AgentEvent::new(
        request.parent_id,
        3,
        2,
        batch,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
            model: ModelIdentity {
                provider_id: "synthetic".into(),
                model_id: "pending".into(),
                revision: None,
            },
        },
    )
    .unwrap();
    db.write(
        JournalCommand::new(request.parent_id, batch, 2, vec![event], None, 2).unwrap(),
        research_data_runtime::WriteOptions::default(),
    )
    .unwrap();
    let before = db
        .owner
        .as_ref()
        .unwrap()
        .inspection_guard()
        .unwrap()
        .snapshot();
    assert!(before.accepted_position > before.materialized_position);
    let changes = db.raw.lock().unwrap().total_changes();
    let catalog = SqliteConversationCatalog::new(db.clone());
    let failure = catalog
        .fork_in_epoch(request.clone(), Uuid::new_v4())
        .await
        .unwrap_err();
    let wire = serde_json::to_value(
        crate::agent::catalog_commands::CatalogMutationCommandError::from(failure),
    )
    .unwrap();
    assert_eq!(wire["reason"]["code"], "DATA_SQL_OPERATION_CONFLICT");
    assert_eq!(wire["reason"]["reason"]["safeCause"], "data_epoch_changed");
    assert_eq!(
        wire["reason"]["reason"]["operationId"],
        request.child_id.as_uuid().to_string()
    );
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    assert_eq!(
        db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot(),
        before
    );
    assert_absent(&db, &prepared);
    let child = catalog.fork_in_epoch(request.clone(), epoch).await.unwrap();
    assert_eq!(child.parent_end_seq, 2);
    assert_eq!(child.conversation.durable_count, 2);
    assert!(
        matches!(db.fork_status(&request).unwrap(), SqlImmediateStatus::Committed(proof) if proof.epoch == epoch)
    );
}

#[tokio::test]
async fn fork_epoch_fence_also_guards_receipt_replay_and_rejects_nil_before_any_write() {
    let (_root, db, prepared) = fixture().await;
    let request = prepared.request().clone();
    db.fork_conversation(&prepared).unwrap();
    let catalog = SqliteConversationCatalog::new(db.clone());
    let changes = db.raw.lock().unwrap().total_changes();
    assert!(
        catalog
            .fork_in_epoch(request.clone(), Uuid::new_v4())
            .await
            .is_err()
    );
    assert!(
        catalog
            .fork_in_epoch(request.clone(), Uuid::nil())
            .await
            .is_err()
    );
    assert_eq!(db.raw.lock().unwrap().total_changes(), changes);
    assert_eq!(
        count(&db, "agent_items", "conversation_id", request.child_id),
        2
    );
}
