//! Upgrade both deployed v136 definitions without rewriting history or hashes.
use super::*;
use sha2::{Digest, Sha256};

const ORIGINAL: &str = include_str!("../../../migrations/0136_agent_context_fragments.sql");
const HARDENED: &str = include_str!("fixtures/0136_hardened.sql");
const ORIGINAL_HASH: &str = "bd3be6e33b9eac03736470114ebf563a55dc8cc698bb64c0c7be038ff4717467";
const HARDENED_HASH: &str = "f720122dafd68e14302352941e8987bea4922bf0c4798fbf99a913e12b7be463";

fn fixture(db: &mut Connection, hardened: bool, maximum: i64) {
    migrate_through(db, 135);
    for &(version, sql) in MIGRATIONS {
        if version <= 135 || version > maximum {
            continue;
        }
        db.execute_batch(if version == 136 && hardened {
            HARDENED
        } else {
            sql
        })
        .unwrap();
        db.execute(
            "insert into schema_migrations(version,applied_at) values(?1,'now')",
            [version],
        )
        .unwrap();
    }
    crate::storage_integrity::backfill_migration_checksums(db).unwrap();
    if hardened {
        db.execute(
            "update migration_checksums set sha256=?1 where version=136",
            [HARDENED_HASH],
        )
        .unwrap();
    }
    db.execute_batch(
        "insert into agent_conversations(id,title) values('conversation','Existing');
         insert into agent_items(conversation_id,ordinal,schema_version,payload_json)
             values('conversation',0,1,'{}');
         insert into agent_context_fragments
             (conversation_id,window_id,boundary_seq,turn_id,anchor_item_id,fragments_json)
             values('conversation','window',1,'turn','anchor','[\"original context\"]');",
    )
    .unwrap();
}

fn checksum(db: &Connection) -> String {
    db.query_row(
        "select sha256 from migration_checksums where version=136",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn deployed_136_bytes_are_pinned() {
    assert_eq!(format!("{:x}", Sha256::digest(ORIGINAL)), ORIGINAL_HASH);
    assert_eq!(format!("{:x}", Sha256::digest(HARDENED)), HARDENED_HASH);
}

#[test]
fn both_deployed_schemas_preserve_context_and_checksums_across_upgrade_and_reopen() {
    for hardened in [false, true] {
        for maximum in [136, 141] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("upgrade.sqlite3");
            let mut db = Connection::open(&path).unwrap();
            fixture(&mut db, hardened, maximum);
            let recorded = checksum(&db);
            crate::storage::migrate(&mut db).unwrap();
            drop(db);
            let mut db = Connection::open(&path).unwrap();
            crate::storage::migrate(&mut db).unwrap();
            assert_eq!(checksum(&db), recorded);
            let (body, ordinal): (String, i64) = db
                .query_row(
                    "select fragments_json,boundary_ordinal from agent_context_fragments",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(body, "[\"original context\"]");
            assert_eq!(ordinal, 0);
            assert!(
                db.execute("update agent_context_fragments set turn_id='changed'", [])
                    .is_err()
            );
            assert!(
                db.execute(
                    "insert into agent_context_fragments
                 (conversation_id,window_id,boundary_seq,turn_id,anchor_item_id,fragments_json)
                 values('conversation','window',2,'turn','anchor','[]')",
                    [],
                )
                .is_err()
            );
            db.execute(
                "delete from agent_items where conversation_id='conversation'",
                [],
            )
            .unwrap();
            assert_eq!(
                db.query_row("select count(*) from agent_context_fragments", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}

#[test]
fn orphaned_boundary_rolls_back_upgrade_without_discarding_context() {
    let mut db = Connection::open_in_memory().unwrap();
    fixture(&mut db, false, 141);
    db.execute("delete from agent_items", []).unwrap();
    assert!(matches!(
        crate::storage::migrate(&mut db),
        Err(crate::storage::StorageError::MigrationIntegrity(_))
    ));
    assert_eq!(
        db.query_row(
            "select fragments_json from agent_context_fragments",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "[\"original context\"]"
    );
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=142",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(checksum(&db), ORIGINAL_HASH);
    assert_eq!(
        db.query_row("pragma foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn unknown_checksum_is_rejected_before_forward_upgrade() {
    let mut db = Connection::open_in_memory().unwrap();
    fixture(&mut db, false, 141);
    db.execute(
        "update migration_checksums set sha256=?1 where version=136",
        ["0".repeat(64)],
    )
    .unwrap();
    assert!(matches!(
        crate::storage::migrate(&mut db),
        Err(crate::storage::StorageError::MigrationIntegrity(_))
    ));
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=142",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn known_hardened_hash_does_not_authorize_other_source_edits() {
    let mut db = Connection::open_in_memory().unwrap();
    fixture(&mut db, true, 141);
    crate::storage_integrity::verify_recorded_migration_checksum(&db, 136, ORIGINAL).unwrap();
    let changed = format!("{ORIGINAL}\n-- unrelated edit\n");
    assert!(
        crate::storage_integrity::verify_recorded_migration_checksum(&db, 136, &changed).is_err()
    );
    assert_eq!(checksum(&db), HARDENED_HASH);
}
