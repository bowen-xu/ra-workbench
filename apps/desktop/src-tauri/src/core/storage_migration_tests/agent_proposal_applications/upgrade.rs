//! Both deployed v114 definitions must upgrade without rewriting their receipts.
use super::*;
use sha2::{Digest, Sha256};

const ORIGINAL_HASH: &str = "db9b6eea85eea41d6e8658d2bf058980f0827b678815cde191bfe089b366c435";
const HARDENED_HASH: &str = "42f00e2d7ebec85129f493ad665cff5e13a6b22cdb4fcfff10fcc97bf9e985f5";
const HARDENED_SQL: &str = include_str!("../fixtures/0114_hardened.sql");
const CHECKPOINT: &str = include_str!(
    "../../../../../../../packages/writing/fixtures/document-checkpoint-editor-receipt-v1.json"
);

fn original_sql() -> String {
    HARDENED_SQL.replace(
        concat!(
            "    not exists(select 1 from agent_change_set_files c\n",
            "        where c.change_set_id=old.change_set_id and c.relative_path=old.relative_path\n",
            "        and c.before_sha256 is not null and c.after_sha256 is not null\n",
            "        and c.before_executable=c.after_executable) or\n",
        ),
        "",
    )
}

fn migrate_to_114(db: &mut Connection) {
    for &(version, sql) in crate::storage_migrations::MIGRATIONS {
        if version <= 80 || version > 114 {
            continue;
        }
        let rebuild =
            crate::storage_migrations::MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED.contains(&version);
        if rebuild {
            db.execute_batch("pragma foreign_keys=off").unwrap();
        }
        let tx = db.transaction().unwrap();
        tx.execute_batch(sql).unwrap();
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
    crate::storage_integrity::backfill_migration_checksums(db).unwrap();
}
fn fixture_114() -> Connection {
    let mut db = super::super::agent_change_sets::fixture();
    for path in ["main.tex", "other.tex"] {
        super::super::agent_change_sets::add_file(&db, path);
    }
    migrate_to_114(&mut db);
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('op','set','apply')", []).unwrap();
    db
}

fn previous_database(hardened: bool) -> Connection {
    let db = fixture_114();
    target(&db);
    db.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values('00000000-0000-4000-8000-000000000001','p','Checkpoint fixture','now','now')", []).unwrap();
    db.execute("insert into writing_document_checkpoints(manuscript_id,relative_path,revision,checkpoint_json,updated_at) values('00000000-0000-4000-8000-000000000001','sections/main.tex',3,?1,'now')", [CHECKPOINT]).unwrap();
    install_deployed_definition(&db, hardened);
    db
}

fn install_deployed_definition(db: &Connection, hardened: bool) {
    // Reproduce the deployed v114 schema and journal, rather than changing only
    // its checksum. No future migration is recorded in this fixture.
    db.execute_batch(
        "delete from migration_checksums where version > 114;
         delete from schema_migrations where version > 114;
         drop trigger agent_proposal_application_files_target_rebind;",
    )
    .unwrap();
    let sql = if hardened {
        HARDENED_SQL.into()
    } else {
        original_sql()
    };
    let hash = if hardened {
        HARDENED_HASH
    } else {
        ORIGINAL_HASH
    };
    assert_eq!(format!("{:x}", Sha256::digest(sql.as_bytes())), hash);
    db.execute_batch(&sql).unwrap();
    db.execute(
        "update migration_checksums set sha256=?1 where version=114",
        [hash],
    )
    .unwrap();
}

#[test]
fn original_migration_114_checksum_stays_stable() {
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_str!(
                "../../../../migrations/0114_pending_application_editor_targets.sql"
            ))
        ),
        ORIGINAL_HASH
    );
}

#[test]
fn both_deployed_114_databases_upgrade_and_reopen_without_losing_pending_work() {
    for hardened in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("upgrade.sqlite3");
        let db = previous_database(hardened);
        db.execute("vacuum into ?1", [path.to_str().unwrap()])
            .unwrap();
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        let backup = crate::storage_backup::backup_before_pending_migrations(
            &db,
            &directory.path().join("backups"),
        )
        .unwrap()
        .expect("back up v114 before repair");
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(db.query_row("select checkpoint_json from writing_document_checkpoints where relative_path='sections/main.tex'", [], |r| r.get::<_, String>(0)).unwrap(), CHECKPOINT);
        assert_eq!(
            db.query_row("select max(version) from schema_migrations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            crate::storage_migrations::MIGRATIONS.last().unwrap().0
        );
        assert_eq!(
            db.query_row(
                "select sha256 from migration_checksums where version=114",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            if hardened {
                HARDENED_HASH
            } else {
                ORIGINAL_HASH
            }
        );
        assert_eq!(
            db.query_row("select title from manuscripts where id='m'", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "m"
        );
        assert_eq!(db.query_row("select phase from agent_proposal_application_files where application_id='op' and relative_path='main.tex'", [], |r| r.get::<_, String>(0)).unwrap(), "pending");
        assert_eq!(
            db.query_row("select count(*) from content_objects", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
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
        assert_eq!(
            backup
                .query_row("select max(version) from schema_migrations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            114
        );
        drop(db);
        let mut reopened = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut reopened).unwrap();
    }
}

#[test]
fn repair_preserves_recovery_but_rejects_non_text_rebindings_on_both_schemas() {
    for hardened in [false, true] {
        for (before, after, after_executable, allowed) in [
            (Some("a".repeat(64)), Some("b".repeat(64)), 0, true),
            (None, Some("b".repeat(64)), 0, false),
            (Some("a".repeat(64)), None, 0, false),
            (Some("a".repeat(64)), Some("b".repeat(64)), 1, false),
        ] {
            let mut db = super::super::agent_change_sets::fixture();
            db.execute(
                "insert into agent_change_set_files(change_set_id,relative_path,
                    before_sha256,after_sha256,before_executable,after_executable,source_json)
                 values('set','main.tex',?1,?2,?3,?4,?5)",
                rusqlite::params![
                    before,
                    after,
                    before.as_ref().map(|_| 0),
                    after.as_ref().map(|_| after_executable),
                    before.as_ref().map(|_| "{}")
                ],
            )
            .unwrap();
            migrate_to_114(&mut db);
            db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('op','set','apply')", []).unwrap();
            target(&db);
            install_deployed_definition(&db, hardened);
            crate::storage::migrate(&mut db).unwrap();
            db.execute(
                "update agent_proposal_applications set status='recovery_required' where id='op'",
                [],
            )
            .unwrap();
            let result = db.execute(
                "update agent_proposal_application_files set target_kind='editor',
                    expected_editor_session='11111111-1111-4111-8111-111111111111',
                    expected_editor_revision='22222222-2222-4222-8222-222222222222'
                 where application_id='op' and relative_path='main.tex'",
                [],
            );
            assert_eq!(
                result.is_ok(),
                allowed,
                "hardened={hardened}, allowed={allowed}"
            );
        }
    }
}

#[test]
fn unknown_114_checksum_is_rejected_before_repair() {
    let mut db = previous_database(false);
    db.execute(
        "update migration_checksums set sha256=?1 where version=114",
        ["0".repeat(64)],
    )
    .unwrap();
    let error = crate::storage::migrate(&mut db).unwrap_err();
    assert!(matches!(
        error,
        crate::storage::StorageError::MigrationIntegrity(_)
    ));
    assert!(error.to_string().contains("迁移 114"));
    assert_eq!(
        db.query_row("select max(version) from schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        114
    );
}

#[test]
fn known_114_checksum_does_not_allow_further_canonical_edits() {
    let db = previous_database(true);
    let changed = format!("{}\n-- unexpected edit\n", original_sql());
    assert!(
        crate::storage_integrity::verify_recorded_migration_checksum(&db, 114, &changed).is_err()
    );
}

#[test]
fn failed_repair_transaction_preserves_old_trigger_and_migration_record() {
    let mut db = previous_database(false);
    let old: String = db.query_row("select sql from sqlite_master where name='agent_proposal_application_files_target_rebind'", [], |r| r.get(0)).unwrap();
    db.execute_batch("create trigger fail_repair before insert on schema_migrations when new.version=115 begin select raise(abort,'fixture repair failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(db.query_row("select sql from sqlite_master where name='agent_proposal_application_files_target_rebind'", [], |r| r.get::<_, String>(0)).unwrap(), old);
    assert_eq!(
        db.query_row(
            "select count(*) from migration_checksums where version=115",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("drop trigger fail_repair;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
}

#[test]
fn repair_migration_keeps_its_own_integrity_check() {
    let mut db = fixture();
    db.execute(
        "update migration_checksums set sha256=?1 where version=115",
        [HARDENED_HASH],
    )
    .unwrap();
    let error = crate::storage::migrate(&mut db).unwrap_err();
    assert!(error.to_string().contains("迁移 115"));
}

#[test]
fn pending_undo_migration_keeps_its_own_integrity_check() {
    let mut db = fixture();
    db.execute(
        "update migration_checksums set sha256=?1 where version=116",
        [HARDENED_HASH],
    )
    .unwrap();
    let error = crate::storage::migrate(&mut db).unwrap_err();
    assert!(error.to_string().contains("迁移 116"));
}
