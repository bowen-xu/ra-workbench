use super::*;

#[test]
fn paired_210_upgrade_keeps_exact_legacy_queue_receipt_and_pending_suffix_without_fabricating_proof()
 {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("queue.sqlite3");
    let log = root.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut sql, 210);
    crate::storage_integrity::backfill_migration_checksums(&sql).unwrap();
    let c = Uuid::new_v4();
    let r = Uuid::new_v4();
    sql.execute(
        "insert into agent_conversations(id,title) values(?1,'Synthetic queue edit')",
        [c.to_string()],
    )
    .unwrap();
    let edit = QueueEdit::Add(QueuedInput {
        id: Uuid::new_v4(),
        text: "old synthetic queue".into(),
        user_input: vec![],
        origin: None,
    });
    let saved = input_queue::apply_checked(&mut sql, c, r, 0, edit.clone(), |_| Ok(())).unwrap();
    let command: String = sql
        .query_row(
            "select command_json from agent_input_queue_receipts where request_id=?1",
            [r.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let checksums: Vec<(u64, String)> = sql
        .prepare("select version,sha256 from migration_checksums order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let db = Database::open(sql, &log).unwrap();
    let intent = QueueEditIntent {
        conversation_id: c,
        request_id: r,
        expected_revision: 0,
        expected_epoch: None,
        scope: QueueEditScope::Context,
        edit,
    };
    let f = Fixture { root, db, intent };
    ordering::accept_pending(&f);
    assert_eq!(
        f.observer()
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    let Fixture { root, db, intent } = f;
    drop(db);
    let sql = Connection::open(&path).unwrap();
    let locked = Database::lock_existing_pair(&sql, &log).unwrap().unwrap();
    let (mut sql, locked) = Database::prepare_schema_migration(sql, &log, locked).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let after: Vec<(u64, String)> = sql
        .prepare(
            "select version,sha256 from migration_checksums where version<=210 order by version",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(after, checksums);
    let db = Database::open_with_log(sql, &log, Some(locked)).unwrap();
    let f = Fixture { root, db, intent };
    assert!(
        matches!(f.db.input_queue_edit_status(&f.intent).unwrap(),QueueEditStatus::Unconfirmed(detail) if detail.safe_cause=="legacy_queue_edit_receipt_without_sql_proof")
    );
    assert!(
        matches!(f.db.edit_input_queue(&f.intent).unwrap(),QueueEditOutcome::LegacyReplay(queue) if queue==saved)
    );
    assert_eq!(f.count(), (1, 0));
    assert_eq!(
        f.observer()
            .query_row(
                "select command_json from agent_input_queue_receipts where request_id=?1",
                [r.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        command
    );
    assert_eq!(
        f.observer()
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        1
    );
    let f = f.reopen();
    assert!(
        matches!(f.db.edit_input_queue(&f.intent).unwrap(),QueueEditOutcome::LegacyReplay(queue) if queue==saved)
    );
    assert_eq!(f.count(), (1, 0));
}
