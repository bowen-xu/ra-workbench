use super::*;

pub(super) fn accept_pending(f: &Fixture) {
    use crate::data_interface::journal::JournalCommand;
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, TaskKind, TurnOutcome},
    };
    let c = ConversationId::from_uuid(f.intent.conversation());
    let r = Uuid::new_v4();
    let turn = Uuid::new_v4();
    let payloads = [
        EventPayload::TurnStarted {
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            model: research_agent_core::protocol::ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "pending".into(),
                revision: None,
            },
            task_kind: TaskKind::Regular,
        },
        EventPayload::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
        },
    ];
    let events = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| AgentEvent::new(c, index as u64 + 1, 0, r, payload).unwrap())
        .collect();
    let accepted =
        f.db.write(
            JournalCommand::new(c, Uuid::new_v4(), 0, events, None, 1_790_000_000_000).unwrap(),
            research_data_runtime::WriteOptions::default(),
        )
        .unwrap();
    assert_eq!(accepted.position, 1);
}

#[test]
fn status_does_not_materialize_pending_history_and_fresh_prepare_commits_after_the_same_prefix() {
    let f = Fixture::new(false);
    let original = f.prepare().unwrap();
    accept_pending(&f);
    let head = || {
        f.db.raw
            .lock()
            .unwrap()
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap()
    };
    let before = f.db.raw.lock().unwrap().total_changes();
    assert_eq!(head(), 0);
    let WritingPrepareStatus::Committed { record, receipt } =
        f.db.writing_prepare_status(&f.intent).unwrap()
    else {
        panic!("old immutable proof required")
    };
    assert_eq!(*record, original);
    assert_eq!(receipt.sql_fence.position, 0);
    let mut fresh = f.intent.clone();
    fresh.request.request_id = Uuid::new_v4();
    fresh.request.retrieval = None;
    assert!(matches!(
        f.db.writing_prepare_status(&fresh).unwrap(),
        WritingPrepareStatus::Unconfirmed(_)
    ));
    assert_eq!(head(), 0);
    assert_eq!(f.db.raw.lock().unwrap().total_changes(), before);
    let mut source = f.source();
    source.request = fresh.request.clone().into();
    f.db.prepare_writing(&fresh, source).unwrap();
    assert_eq!(head(), 1);
    assert_eq!(f.count("agent_items"), 2);
    let WritingPrepareStatus::Committed { receipt, .. } =
        f.db.writing_prepare_status(&fresh).unwrap()
    else {
        panic!("fresh proof required")
    };
    assert_eq!(receipt.sql_fence.position, 1);
    assert_eq!(
        receipt.epoch,
        f.db.owner
            .as_ref()
            .unwrap()
            .inspection_guard()
            .unwrap()
            .snapshot()
            .epoch
    );
}

#[test]
fn paired_backup_restores_original_prepare_proof_with_the_accepted_unprojected_suffix() {
    let f = Fixture::new(true);
    let original = f.prepare().unwrap();
    let WritingPrepareStatus::Committed { receipt, .. } =
        f.db.writing_prepare_status(&f.intent).unwrap()
    else {
        panic!("proof required")
    };
    accept_pending(&f);
    std::fs::remove_file(
        f.store
            .manuscript_directory()
            .join(f.intent.manuscript().to_string())
            .join("source/main.tex"),
    )
    .unwrap();
    // The ordinary startup backup owns the same exclusive engine lock before
    // publishing an owner. Do not shutdown/materialize away the pending suffix.
    let Fixture {
        root,
        db,
        store,
        intent,
    } = f;
    drop(store);
    drop(db);
    let source = Connection::open(root.path().join("prepare.sqlite3")).unwrap();
    let prefix = source
        .query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap();
    assert_eq!(prefix, 0);
    let locked = Database::lock_existing_pair(&source, &root.path().join("log"))
        .unwrap()
        .unwrap();
    assert_eq!(locked.head().unwrap().accepted_position, 1);
    assert_eq!(locked.head().unwrap().epoch, receipt.epoch);
    let backup = crate::storage_backup::backup_pair_for_next_schema_test(
        &source,
        &root.path().join("backups"),
        &locked,
    )
    .unwrap()
    .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(backup.with_extension("pair.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["complete"], true);
    assert_eq!(manifest["epoch"], receipt.epoch.to_string());
    assert_eq!(manifest["acceptedPosition"], 1);
    assert_eq!(manifest["materializedPosition"], 0);
    let backup_sql = Connection::open(&backup).unwrap();
    let backup_log = backup.with_extension("raft");
    let restored_lock = Database::lock_existing_pair(&backup_sql, &backup_log)
        .unwrap()
        .unwrap();
    let (sql, restored_lock) =
        Database::prepare_schema_migration(backup_sql, &backup_log, restored_lock).unwrap();
    let restored = Database::open_with_log(sql, &backup_log, Some(restored_lock)).unwrap();
    let WritingPrepareStatus::Committed {
        record,
        receipt: restored_receipt,
    } = restored.writing_prepare_status(&intent).unwrap()
    else {
        panic!("restored proof required")
    };
    assert_eq!(*record, original);
    assert_eq!(restored_receipt.epoch, receipt.epoch);
    assert_eq!(restored_receipt.sql_fence, receipt.sql_fence);
    assert_eq!(restored_receipt.fingerprint, receipt.fingerprint);
    assert_eq!(
        restored.replay_writing_prepare(&intent).unwrap(),
        Some(original)
    );
    let db = restored.raw.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select materialized_position from data_runtime_state",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "select durable_count from agent_conversations where id=?1",
            [intent.conversation().to_string()],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("select count(*) from agent_submissions", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "select count(*) from data_sql_operation_receipts",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    restored.shutdown().unwrap();
}
