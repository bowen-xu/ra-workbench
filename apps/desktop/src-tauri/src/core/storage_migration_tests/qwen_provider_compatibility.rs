use super::*;

fn providers(db: &Connection) -> Vec<String> {
    db.prepare("select json_array(provider_id,provider_kind,enabled,base_url,region,model_id,thinking_mode,timeout_ms,updated_at) from ai_provider_settings where provider_id!='qwen' order by provider_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn checksums(db: &Connection) -> Vec<(i64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=231 order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn configuration(db: &Connection) -> Vec<String> {
    db.prepare("select json_array(provider_id,enabled,base_url,timeout_ms,updated_at) from llm_provider_configs order by provider_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn insert_reference(db: &Connection) -> rusqlite::Result<usize> {
    db.execute(
        "insert into qwen_compatibility_probe(id,provider_id) values('qwen-reference','qwen')",
        [],
    )
}

#[test]
fn qwen_provider_upgrade_preserves_existing_rows_configuration_checksums_and_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 231);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("create table qwen_compatibility_probe(id text primary key,provider_id text not null references ai_provider_settings(provider_id));
        insert into qwen_compatibility_probe values('old-reference','deepseek');").unwrap();
    let before = providers(&db);
    let settings = configuration(&db);
    let hashes = checksums(&db);
    assert_eq!(
        insert_reference(&db)
            .unwrap_err()
            .sqlite_error()
            .unwrap()
            .extended_code,
        787
    );
    crate::storage::migrate(&mut db).unwrap();
    insert_reference(&db).unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(providers(&db), before);
    assert_eq!(configuration(&db), settings);
    assert_eq!(checksums(&db), hashes);
    let compatibility: (String, i64, String, String) = db.query_row(
        "select provider_kind,enabled,base_url,model_id from ai_provider_settings where provider_id='qwen'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    assert_eq!(
        compatibility,
        ("generation".into(), 0, String::new(), String::new())
    );
    assert_eq!(
        db.query_row("select count(*) from qwen_compatibility_probe", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("pragma integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn qwen_provider_upgrade_failure_rolls_back_and_restores_foreign_keys() {
    let mut db = Connection::open_in_memory().unwrap();
    migrate_through(&mut db, 231);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("create table qwen_compatibility_probe(id text primary key,provider_id text not null references ai_provider_settings(provider_id));
        pragma foreign_keys=off;
        insert into qwen_compatibility_probe values('orphan','missing');
        pragma foreign_keys=on;").unwrap();
    let before = providers(&db);
    let hashes = checksums(&db);
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(providers(&db), before);
    assert_eq!(checksums(&db), hashes);
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=232",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from ai_provider_settings where provider_id='qwen'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("pragma foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute("delete from qwen_compatibility_probe where id='orphan'", [])
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    insert_reference(&db).unwrap();
}
