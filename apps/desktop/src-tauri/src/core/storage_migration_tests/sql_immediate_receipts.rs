use super::migrate_through;
use rusqlite::{Connection, params};
use uuid::Uuid;

#[path = "sql_immediate_receipts/paired.rs"]
mod paired;

fn latest(db: &Connection) -> i64 {
    db.query_row("select max(version) from schema_migrations", [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn receipt(db: &Connection, operation: Uuid, fence: i64) -> rusqlite::Result<usize> {
    db.execute(
        "insert into data_sql_operation_receipts values(?1,?2,'identity.fixture',1,?3,?4,'null')",
        params![
            operation.to_string(),
            Uuid::new_v4().to_string(),
            [5_u8; 32].as_slice(),
            fence
        ],
    )
}

fn batch(db: &Connection, conversation: &str, batch: &str) -> rusqlite::Result<usize> {
    db.execute(
        "insert into agent_journal_batches values(?1,?2,0,1,'retained-legacy-hash')",
        params![conversation, batch],
    )
}

fn conversation(db: &Connection, id: &str) {
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'legacy identity')",
        [id],
    )
    .unwrap();
}

fn schema(db: &Connection) -> Vec<(String, String, Option<String>)> {
    db.prepare("select type,name,sql from sqlite_schema order by type,name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn real_210_upgrade_backfills_all_legacy_operations_and_preserves_frozen_hashes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 210);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let b = Uuid::new_v4();
    let ids = [Uuid::new_v4(), Uuid::new_v4()];
    for c in ids {
        conversation(&db, &c.to_string());
        batch(&db, &c.to_string(), &b.to_string()).unwrap();
    }
    let before: Vec<(i64, String)> = db
        .prepare("select version,sha256 from migration_checksums order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        db.query_row("select count(*) from data_operation_receipts", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        latest(&db),
        crate::storage_migrations::MIGRATIONS.last().unwrap().0
    );
    let indexed:Vec<(String,String,String)>=db.prepare("select conversation_id,batch_id,operation_id from data_journal_operation_ids order by conversation_id").unwrap()
        .query_map([],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(indexed.len(), 2);
    for (c, old_batch, operation) in &indexed {
        assert_eq!(old_batch, &b.to_string());
        assert_eq!(
            operation,
            &Uuid::new_v5(&Uuid::parse_str(c).unwrap(), b.as_bytes()).to_string()
        );
        assert!(receipt(&db, Uuid::parse_str(operation).unwrap(), 0).is_err());
    }
    let after: Vec<(i64, String)> = db
        .prepare(
            "select version,sha256 from migration_checksums where version<=210 order by version",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(
        db.query_row(
            "select count(*) from agent_journal_batches where payload_hash='retained-legacy-hash'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, u64>(0)
        })
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        latest(&db),
        crate::storage_migrations::MIGRATIONS.last().unwrap().0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from data_journal_operation_ids",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn invalid_old_uuid_rolls_back_the_real_211_and_reopen_keeps_legacy_evidence() {
    for (c, b) in [
        (
            "invalid-old-conversation".to_owned(),
            Uuid::new_v4().to_string(),
        ),
        (Uuid::new_v4().to_string(), "invalid-old-batch".to_owned()),
        (Uuid::nil().to_string(), Uuid::new_v4().to_string()),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("invalid.sqlite3");
        let mut db = Connection::open(&path).unwrap();
        migrate_through(&mut db, 210);
        crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
        conversation(&db, &c);
        batch(&db, &c, &b).unwrap();
        let before = schema(&db);
        for _ in 0..2 {
            let error = crate::storage::migrate(&mut db).unwrap_err().to_string();
            assert!(error.contains("DATA_JOURNAL_IDENTITY_INVALID"));
            assert!(!error.contains("invalid-old-conversation"));
            assert!(!error.contains("invalid-old-batch"));
            assert_eq!(latest(&db), 210);
            assert_eq!(schema(&db), before);
            assert_eq!(
                db.query_row("pragma foreign_keys", [], |row| row.get::<_, u64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                db.query_row(
                    "select count(*) from migration_checksums where version=211",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                db.query_row(
                    "select payload_hash from agent_journal_batches",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
                "retained-legacy-hash"
            );
            drop(db);
            db = Connection::open(&path).unwrap();
        }
    }
}

#[test]
fn sql_receipts_use_zero_or_shared_fences_and_both_namespace_guards_are_atomic() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    receipt(&db, Uuid::new_v4(), 0).unwrap();
    receipt(&db, Uuid::new_v4(), 0).unwrap();
    assert!(receipt(&db, Uuid::new_v4(), -1).is_err());
    let c = Uuid::new_v4();
    let b = Uuid::new_v4();
    let operation = Uuid::new_v5(&c, b.as_bytes());
    conversation(&db, &c.to_string());
    receipt(&db, operation, 0).unwrap();
    assert!(batch(&db, &c.to_string(), &b.to_string()).is_err());
    assert_eq!(
        db.query_row("select count(*) from agent_journal_batches", [], |row| row
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from data_journal_operation_ids",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let epoch = Uuid::new_v4();
    assert!(
        db.execute(
            "insert into data_operation_receipts values(?1,?2,1,?3,'1')",
            params![
                operation.to_string(),
                epoch.to_string(),
                [8_u8; 32].as_slice()
            ]
        )
        .is_err()
    );
    let log_operation = Uuid::new_v4();
    db.execute(
        "insert into data_operation_receipts values(?1,?2,1,?3,'1')",
        params![
            log_operation.to_string(),
            epoch.to_string(),
            [8_u8; 32].as_slice()
        ],
    )
    .unwrap();
    assert!(receipt(&db, log_operation, 0).is_err());
    let future = Uuid::new_v4();
    {
        let tx = db.transaction().unwrap();
        batch(&tx, &c.to_string(), &future.to_string()).unwrap();
        assert_eq!(
            tx.query_row(
                "select count(*) from data_journal_operation_ids",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        tx.rollback().unwrap();
    }
    assert_eq!(
        db.query_row(
            "select count(*) from data_journal_operation_ids",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert!(db.execute("insert into data_sql_operation_receipts values(?1,?2,'identity.fixture',1,x'00',0,'null')",params![Uuid::new_v4().to_string(),epoch.to_string()]).is_err());
}
