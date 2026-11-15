use super::*;
use sha2::{Digest, Sha256};

const EARLY: &str = "4c772a1ce2bccc8ac95feebf4e0437b4518cedf4dc4f53cec03c5738ec8d7482";
const CURRENT: &str = "0ae1becddfb7faa5f226352dc3004de5ebb68692a29b81ca3082912dece2e087";

#[test]
#[ignore = "Internal verification entry; requires an explicitly supplied disposable database backup"]
fn reading_source_retention_upgrades_disposable_deployed_backup() {
    let path = std::env::var("RA_RETENTION_TEST_BACKUP").unwrap();
    let mut db = Connection::open(&path).unwrap();
    let before = note_fingerprint(&db);
    let recorded = hash(&db);
    crate::storage::migrate(&mut db).unwrap();
    assert!(
        note_fingerprint(&db) == before,
        "note/source payload changed"
    );
    assert_eq!(hash(&db), recorded);
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert!(
        note_fingerprint(&db) == before,
        "reopened note/source payload changed"
    );
    assert_eq!(hash(&db), recorded);
    let version: i64 = db
        .query_row("select max(version) from schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, MIGRATIONS.last().unwrap().0);
}

fn note_fingerprint(db: &Connection) -> Vec<Vec<(String, String, String)>> {
    [
        "select id,content_json,plain_text from note_blocks order by id",
        "select block_id,page_id,quote_exact from reading_note_sources order by block_id",
        "select id,title,status from note_pages order by id",
    ]
    .into_iter()
    .map(|sql| {
        db.prepare(sql)
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
    .collect()
}

fn fixture(early: bool) -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    migrate_through(&mut db, 143);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    if early {
        // Reconstruct the observed deployed catalog, not the unavailable early
        // SQL bytes: ordinary page/source schema, without the retention suffix.
        let view: String = db
            .query_row(
                "select sql from sqlite_master where name='content_object_references'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let original = view
            .rsplit_once("union all\nselect content_sha256, 'reading_note_source'")
            .unwrap()
            .0;
        db.execute_batch(
            "drop trigger content_object_gc_reading_source_insert;
            drop trigger content_object_gc_reading_source_delete;
            drop view content_object_references;",
        )
        .unwrap();
        db.execute_batch(original).unwrap();
        db.execute(
            "update migration_checksums set sha256=?1 where version=143",
            [EARLY],
        )
        .unwrap();
    }
    db.execute_batch("insert into note_pages(id,title,created_at,updated_at) values('page','Retained document','now','now');
        insert into reading_note_page_map(legacy_id,page_id) values('alias','page');
        insert into reading_note_sources(block_id,page_id,note_type,color,paper_id_snapshot,paper_title_snapshot,
            attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,rects_json,
            quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at)
        values('alias','page','annotation','#f4cf65','missing','Retained title','missing','source.pdf',
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,'[]','Retained quote','','',1,'now','now');").unwrap();
    db
}

fn hash(db: &Connection) -> String {
    db.query_row(
        "select sha256 from migration_checksums where version=143",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn reading_source_retention_freezes_current_143_bytes() {
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../../migrations/0143_reading_note_documents.sql"
            ))
        ),
        CURRENT
    );
}

#[test]
fn reading_source_retention_upgrades_both_known_catalogs_and_preserves_hash_and_sources() {
    for early in [false, true] {
        let mut db = fixture(early);
        let recorded = hash(&db);
        crate::storage::migrate(&mut db).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(hash(&db), recorded);
        let source: (String, String) = db
            .query_row(
                "select page_id,quote_exact from reading_note_sources where block_id='alias'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(source, ("page".into(), "Retained quote".into()));
        let retained: i64 = db.query_row("select count(*) from content_object_references where owner_kind='reading_note_source' and owner_id='page'", [], |row| row.get(0)).unwrap();
        assert_eq!(retained, 1);
        let triggers: i64 = db.query_row("select count(*) from sqlite_master where type='trigger' and name in ('content_object_gc_reading_source_insert','content_object_gc_reading_source_delete')", [], |row| row.get(0)).unwrap();
        assert_eq!(triggers, 2);
        assert_eq!(
            db.prepare("pragma foreign_key_check")
                .unwrap()
                .query_map([], |_| Ok(()))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn reading_source_retention_rejects_unknown_hash_without_publishing_upgrade() {
    let mut db = fixture(true);
    db.execute(
        "update migration_checksums set sha256=?1 where version=143",
        ["0".repeat(64)],
    )
    .unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(hash(&db), "0".repeat(64));
    let installed: i64 = db
        .query_row(
            "select count(*) from schema_migrations where version=144",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(installed, 0);
}

#[test]
fn reading_source_retention_failure_rolls_back_views_triggers_and_migration_record() {
    let mut db = fixture(true);
    db.execute_batch("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
        values('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','synthetic.pdf','application/pdf',1,'now');
        insert or ignore into content_object_gc_candidates(content_sha256,unreferenced_since,not_before,reason)
        values('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','now','later','reconciliation');
        create trigger reject_retention_cleanup before delete on content_object_gc_candidates
        begin select raise(abort,'synthetic cleanup failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(hash(&db), EARLY);
    let installed: i64 = db
        .query_row(
            "select count(*) from schema_migrations where version=144",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(installed, 0);
    let retained: i64 = db
        .query_row(
            "select count(*) from content_object_references where owner_kind='reading_note_source'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, 0);
    let triggers: i64 = db.query_row("select count(*) from sqlite_master where name='content_object_gc_reading_source_insert'", [], |row| row.get(0)).unwrap();
    assert_eq!(triggers, 0);
    db.execute_batch("drop trigger reject_retention_cleanup;")
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
}
