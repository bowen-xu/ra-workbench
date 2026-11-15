//! Preserve both deployed v118 journals while repairing their guard forward.
use super::*;
use sha2::{Digest, Sha256};

const ORIGINAL: &str =
    include_str!("../../../migrations/0118_checkpointed_undo_editor_restorations.sql");
const CORRECTED: &str = include_str!("fixtures/0118_corrected.sql");
const ORIGINAL_HASH: &str = "6be120040299e7033fc2a6568e7b6e7cfe23d37fbaf173f8f5fcccc2be139278";
const CORRECTED_HASH: &str = "e9b9421430411680fdc518034a3cf00c63246bcd7621ec9716cf93520a1932b7";
const CHECKPOINT: &str = include_str!(
    "../../../../../../packages/writing/fixtures/document-checkpoint-editor-receipt-v1.json"
);

fn fixture(corrected: bool, maximum: i64) -> Connection {
    let mut db = super::agent_change_sets::fixture();
    super::agent_change_sets::add_file(&db, "main.tex");
    for &(version, sql) in MIGRATIONS {
        if version <= 80 || version > maximum {
            continue;
        }
        let rebuild = MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED.contains(&version);
        if rebuild {
            db.execute_batch("pragma foreign_keys=off").unwrap();
        }
        let tx = db.transaction().unwrap();
        tx.execute_batch(if version == 118 && corrected {
            CORRECTED
        } else {
            sql
        })
        .unwrap();
        tx.execute(
            "insert into schema_migrations(version,applied_at) values(?1,'now')",
            [version],
        )
        .unwrap();
        tx.commit().unwrap();
        if rebuild {
            db.execute_batch("pragma foreign_keys=on").unwrap();
        }
    }
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute(
        "update migration_checksums set sha256=?1 where version=118",
        [if corrected {
            CORRECTED_HASH
        } else {
            ORIGINAL_HASH
        }],
    )
    .unwrap();
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('op','set','apply')", []).unwrap();
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','main.tex','disk')", []).unwrap();
    db.execute("insert into writing_document_checkpoints(manuscript_id,relative_path,revision,checkpoint_json,updated_at) values('m','main.tex',3,?1,'now')", [CHECKPOINT]).unwrap();
    db
}

fn guard(db: &Connection) -> String {
    db.query_row(
        "select sql from sqlite_master where name='agent_editor_restorations_admission'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn checksum(db: &Connection) -> String {
    db.query_row(
        "select sha256 from migration_checksums where version=118",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn deployed_118_migration_bytes_are_pinned() {
    assert_eq!(format!("{:x}", Sha256::digest(ORIGINAL)), ORIGINAL_HASH);
    assert_eq!(format!("{:x}", Sha256::digest(CORRECTED)), CORRECTED_HASH);
}

#[test]
fn both_deployed_118_databases_upgrade_backup_and_reopen_without_losing_pending_work() {
    for corrected in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("upgrade.sqlite3");
        let db = fixture(corrected, 118);
        let original_guard = guard(&db);
        let hash = checksum(&db);
        db.execute("vacuum into ?1", [path.to_str().unwrap()])
            .unwrap();
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        let backup = crate::storage_backup::backup_before_pending_migrations(
            &db,
            &root.path().join("backups"),
        )
        .unwrap()
        .unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_ne!(guard(&db), original_guard);
        assert_eq!(checksum(&db), hash);
        assert_eq!(
            db.query_row(
                "select checkpoint_json from writing_document_checkpoints",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            CHECKPOINT
        );
        assert_eq!(
            db.query_row(
                "select phase from agent_proposal_application_files where application_id='op'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "pending"
        );
        assert_eq!(
            db.query_row(
                "select source_json from agent_change_set_files where change_set_id='set'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "{}"
        );
        assert_eq!(
            db.query_row("pragma quick_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert_eq!(
            db.query_row("select count(*) from pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        let backup = Connection::open(backup).unwrap();
        assert_eq!(checksum(&backup), hash);
        assert_eq!(guard(&backup), original_guard);
        assert_eq!(
            backup
                .query_row("select max(version) from schema_migrations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            118
        );
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(checksum(&db), hash);
        assert_eq!(
            db.query_row(
                "select count(*) from schema_migrations where version=121",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}

#[test]
fn already_upgraded_120_database_preserves_corrected_118_checksum() {
    let mut db = fixture(true, 120);
    let previous_guard = guard(&db);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(checksum(&db), CORRECTED_HASH);
    assert_eq!(guard(&db), previous_guard);
}

#[test]
fn unknown_118_checksum_stops_before_forward_migrations() {
    let mut db = fixture(false, 118);
    db.execute(
        "update migration_checksums set sha256=?1 where version=118",
        ["0".repeat(64)],
    )
    .unwrap();
    let old = guard(&db);
    assert!(
        crate::storage::migrate(&mut db)
            .unwrap_err()
            .to_string()
            .contains("迁移 118")
    );
    assert_eq!(guard(&db), old);
    assert_eq!(
        db.query_row("select max(version) from schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        118
    );
}

#[test]
fn known_118_hash_does_not_allow_more_edits_or_a_changed_repair() {
    for corrected in [false, true] {
        let mut db = fixture(corrected, 118);
        assert!(
            crate::storage_integrity::verify_recorded_migration_checksum(
                &db,
                118,
                &format!("{ORIGINAL}\n-- unexpected change\n")
            )
            .is_err()
        );
        crate::storage::migrate(&mut db).unwrap();
        db.execute(
            "update migration_checksums set sha256=?1 where version=121",
            [CORRECTED_HASH],
        )
        .unwrap();
        assert!(
            crate::storage::migrate(&mut db)
                .unwrap_err()
                .to_string()
                .contains("迁移 121")
        );
    }
}

#[test]
fn failed_121_repair_rolls_back_guard_and_retries_without_changing_receipts() {
    let mut db = fixture(true, 120);
    db.execute_batch(CORRECTED).unwrap();
    let old = guard(&db);
    db.execute_batch("create trigger fail_121 before insert on schema_migrations when new.version=121 begin select raise(abort,'fixture failure'); end").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(guard(&db), old);
    assert_eq!(checksum(&db), CORRECTED_HASH);
    assert_eq!(
        db.query_row(
            "select count(*) from migration_checksums where version=121",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("drop trigger fail_121").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_ne!(guard(&db), old);
    assert_eq!(
        db.query_row(
            "select checkpoint_json from writing_document_checkpoints",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        CHECKPOINT
    );
    assert_eq!(checksum(&db), CORRECTED_HASH);
}

#[test]
#[ignore = "Retains an empty deployed-v118 database for pnpm dev:desktop startup verification"]
fn native_118_startup_fixture() {
    let root = tempfile::Builder::new()
        .prefix("research-assistant-migration-118-")
        .tempdir()
        .unwrap()
        .keep();
    let mut db = Connection::open(root.join("research-assistant.sqlite3")).unwrap();
    migrate_through(&mut db, 118);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    std::fs::write(root.join("empty-credentials.json"), "{}\n").unwrap();
    println!("NATIVE_118_STARTUP_DIRECTORY={}", root.display());
}
