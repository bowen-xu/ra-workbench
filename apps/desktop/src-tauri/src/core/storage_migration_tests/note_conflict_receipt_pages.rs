use super::*;
use crate::storage_integrity::{backfill_migration_checksums, record_migration_checksum};
use sha2::{Digest, Sha256};

const ORIGINAL: &str = "bb2948706fedca09d9229d43d74772c3911ac3ac7f1ef4cde8c7b24d23573cef";
const IN_PLACE: &str = "7248840b62df4b0e4d6e4ace7eb96f7278f32289d2a4ef0a478548d5ad85fc54";
const IN_PLACE_SQL: &str = include_str!("fixtures/0184_page_fk_in_place.sql");

fn fixture(db: &mut Connection, in_place: bool, through: i64) {
    migrate_through(db, 183);
    backfill_migration_checksums(db).unwrap();
    let sql = if in_place {
        IN_PLACE_SQL
    } else {
        include_str!("../../../migrations/0184_note_conflict_resolutions.sql")
    };
    let tx = db.transaction().unwrap();
    tx.execute_batch(sql).unwrap();
    tx.execute("insert into schema_migrations values(184,'deployed')", [])
        .unwrap();
    record_migration_checksum(&tx, 184, sql).unwrap();
    tx.commit().unwrap();
    db.execute_batch("insert into note_pages(id,title,created_at,updated_at) values('page','Retained note','now','now');
        insert into note_blocks(id,scope_kind,page_id,position,block_type,props_json,content_json,plain_text,schema_version,revision,status,is_favorite,created_at,updated_at)
        values('block','page','page',0,'paragraph','{}','{\"kind\":\"inline\",\"items\":[{\"type\":\"text\",\"text\":\"Retained body\",\"styles\":[]}]}','Retained body',1,0,'active',0,'now','now');
        insert into agent_note_patch_candidates(conversation_id,page_id,proposal_id,generation,baseline_json,candidate_json,state)
        values('conversation','page','proposal',7,'{}','{}','pending');").unwrap();
    let request = "{ \"document\": {\"pageId\":\"page\"}, \"copy\":false }";
    let result = "{ \"sentinel\": \"exact original receipt bytes\" }";
    if in_place {
        db.execute("insert into note_conflict_resolutions(request_id,page_id,request_json,result_json) values('existing','page',?1,?2)", [request, result]).unwrap();
    } else {
        db.execute("insert into note_conflict_resolutions(request_id,request_json,result_json) values('existing',?1,?2)", [request, result]).unwrap();
        db.execute("insert into note_conflict_resolutions(request_id,request_json,result_json) values('orphan','{\"document\":{\"pageId\":\"deleted-page\"}}',?1),('unmapped','{}',?1)", [result]).unwrap();
    }
    for &(version, sql) in MIGRATIONS {
        if version <= 184 || version > through {
            continue;
        }
        assert!(!MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED.contains(&version));
        let tx = db.transaction().unwrap();
        tx.execute_batch(sql).unwrap();
        tx.execute(
            "insert into schema_migrations values(?1,'deployed')",
            [version],
        )
        .unwrap();
        record_migration_checksum(&tx, version, sql).unwrap();
        tx.commit().unwrap();
    }
}

fn snapshot(db: &Connection) -> Vec<String> {
    [
        "select json_array(request_id,request_json,result_json) from note_conflict_resolutions order by request_id",
        "select json_array(id,title,revision,status) from note_pages order by id",
        "select json_array(id,page_id,content_json,plain_text,revision) from note_blocks order by id",
        "select json_array(conversation_id,page_id,proposal_id,generation,baseline_json,candidate_json,state) from agent_note_patch_candidates order by conversation_id,proposal_id",
    ].into_iter().flat_map(|sql| {
        db.prepare(sql).unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap()
    }).collect()
}

fn checksums(db: &Connection) -> Vec<(i64, String)> {
    db.prepare("select version,sha256 from migration_checksums where version<=190 order by version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn catalog(db: &Connection) -> Vec<String> {
    db.prepare("select sql from sqlite_master where tbl_name='note_conflict_resolutions' and sql is not null order by type,name")
        .unwrap().query_map([], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap()
}

fn assert_repaired(db: &Connection, in_place: bool) {
    let page: String = db
        .query_row(
            "select page_id from note_conflict_resolutions where request_id='existing'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(page, "page");
    let unmapped: i64 = db
        .query_row(
            "select count(*) from note_conflict_resolutions where page_id is null",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unmapped, if in_place { 0 } else { 2 });
    assert_eq!(
        db.query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select sha256 from migration_checksums where version=184",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        if in_place { IN_PLACE } else { ORIGINAL }
    );
}

#[test]
fn conflict_receipt_pages_freezes_both_exact_deployed_migrations() {
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../migrations/0184_note_conflict_resolutions.sql"
            ))
        ),
        ORIGINAL
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(IN_PLACE_SQL.as_bytes())),
        IN_PLACE
    );
}

#[test]
fn conflict_receipt_pages_upgrades_both_schemas_and_reopens_without_rewriting_history() {
    let mut target_catalog = None;
    for in_place in [false, true] {
        for through in [184, 190] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("synthetic.sqlite3");
            let mut db = Connection::open(&path).unwrap();
            fixture(&mut db, in_place, through);
            let before = snapshot(&db);
            let hashes = checksums(&db);
            crate::storage::migrate(&mut db).unwrap();
            assert_eq!(snapshot(&db), before);
            assert_repaired(&db, in_place);
            for hash in &hashes {
                assert!(checksums(&db).contains(hash));
            }
            let schema = catalog(&db);
            if let Some(expected) = &target_catalog {
                assert_eq!(&schema, expected);
            } else {
                target_catalog = Some(schema);
            }
            let after_hashes = checksums(&db);
            drop(db);
            let mut reopened = Connection::open(path).unwrap();
            crate::storage::migrate(&mut reopened).unwrap();
            assert_eq!(snapshot(&reopened), before);
            assert_eq!(checksums(&reopened), after_hashes);
            assert_repaired(&reopened, in_place);
        }
    }
}

#[test]
fn conflict_receipt_pages_requires_live_pages_for_new_receipts_and_cascades_deletion() {
    let mut db = Connection::open_in_memory().unwrap();
    fixture(&mut db, false, 190);
    crate::storage::migrate(&mut db).unwrap();
    assert!(db.execute("insert into note_conflict_resolutions(request_id,request_json,result_json) values('null','{}','{}')", []).is_err());
    assert!(
        db.execute(
            "insert into note_conflict_resolutions values('missing','missing','{}','{}')",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            "update note_conflict_resolutions set page_id=null where request_id='existing'",
            []
        )
        .is_err()
    );
    db.execute(
        "insert into note_conflict_resolutions values('new','page','{}','{}')",
        [],
    )
    .unwrap();
    db.execute("delete from note_pages where id='page'", [])
        .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from note_conflict_resolutions where request_id in ('existing','new')",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select count(*) from note_conflict_resolutions",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn conflict_receipt_pages_rolls_back_schema_receipts_and_checksums_on_publish_failure() {
    for in_place in [false, true] {
        let mut db = Connection::open_in_memory().unwrap();
        fixture(&mut db, in_place, 190);
        let before = snapshot(&db);
        let schema = catalog(&db);
        let hashes = checksums(&db);
        db.execute_batch("create trigger fail_receipt_upgrade before insert on schema_migrations when new.version=191 begin select raise(abort,'synthetic failure'); end;").unwrap();
        assert!(crate::storage::migrate(&mut db).is_err());
        assert_eq!(snapshot(&db), before);
        assert_eq!(catalog(&db), schema);
        assert_eq!(checksums(&db), hashes);
        assert_eq!(
            db.query_row(
                "select count(*) from schema_migrations where version=191",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "select count(*) from migration_checksums where version=191",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "select count(*) from sqlite_master where name='note_conflict_resolutions_next'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger fail_receipt_upgrade")
            .unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(snapshot(&db), before);
        assert_repaired(&db, in_place);
    }
}

#[test]
fn conflict_receipt_pages_rejects_unknown_checksum_without_publishing_repair() {
    let mut db = Connection::open_in_memory().unwrap();
    fixture(&mut db, true, 190);
    let before = snapshot(&db);
    let schema = catalog(&db);
    db.execute(
        "update migration_checksums set sha256=?1 where version=184",
        ["0".repeat(64)],
    )
    .unwrap();
    assert!(matches!(
        crate::storage::migrate(&mut db),
        Err(crate::storage::StorageError::MigrationIntegrity(_))
    ));
    assert_eq!(snapshot(&db), before);
    assert_eq!(catalog(&db), schema);
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=191",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "select sha256 from migration_checksums where version=184",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "0".repeat(64)
    );
}
