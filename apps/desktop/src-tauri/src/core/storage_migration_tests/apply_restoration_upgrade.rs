//! Upgrade the restoration guard without altering deployed receipts or checksums.
use super::*;
use rusqlite::params;
use serde_json::json;
use uuid::Uuid;

#[test]
fn apply_restoration_upgrade_is_transactional_and_preserves_acknowledged_owner() {
    let mut db = super::agent_change_sets::fixture();
    super::agent_change_sets::add_file(&db, "main.tex");
    // Build the actual v119 schema, rather than deleting newer registry entries
    // from a current database and leaving their tables behind.
    for &(version, sql) in MIGRATIONS {
        if version <= 80 || version > 119 {
            continue;
        }
        let rebuild = MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED.contains(&version);
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
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch(include_str!("fixtures/0118_corrected.sql"))
        .unwrap();
    let owner = Uuid::new_v4().to_string();
    let revision = Uuid::new_v4().to_string();
    let result = Uuid::new_v4().to_string();
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('op','set','apply')", []).unwrap();
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind,expected_editor_session,expected_editor_revision) values('op','set','main.tex','editor',?1,?2)", params![owner,revision]).unwrap();
    db.execute_batch(
        "update agent_proposal_applications set status='applying';
        update agent_proposal_application_files set phase='dispatched';",
    )
    .unwrap();
    db.execute("update agent_proposal_application_files set phase='acknowledged',result_editor_revision=?1", [&result]).unwrap();
    db.execute(
        "update agent_proposal_applications set status='recovery_required'",
        [],
    )
    .unwrap();
    let row = json!({"content":"frozen result","saveIntent":null,
        "editorReceipt":{"operationId":"op","sessionId":owner,"revision":result,"text":"frozen result"},
        "operations":[{"operationId":"op","conversationId":"c","requestId":"r","resultSha256":"b".repeat(64)}]});
    db.execute("insert into writing_document_checkpoints(manuscript_id,relative_path,revision,checkpoint_json,updated_at) values('m','main.tex',1,?1,'now')", [row.to_string()]).unwrap();
    let target = target_row(&db);
    let old = guard(&db);
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=114",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(insert_restore(&db, &Uuid::new_v4().to_string()).is_err());

    db.execute_batch("create trigger fail_apply_restoration_upgrade before insert on schema_migrations when new.version=120 begin select raise(abort,'fixture failure'); end").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(guard(&db), old);
    assert_eq!(target_row(&db), target);
    assert_eq!(
        db.query_row("select max(version) from schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        119
    );
    assert_eq!(
        db.query_row(
            "select count(*) from migration_checksums where version=120",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("drop trigger fail_apply_restoration_upgrade")
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_ne!(guard(&db), old);
    assert_eq!(target_row(&db), target);
    assert_eq!(
        db.query_row(
            "select sha256 from migration_checksums where version=114",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        checksum
    );
    assert_eq!(
        db.query_row(
            "select checkpoint_json from writing_document_checkpoints",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        row.to_string()
    );

    // Invalid provenance remains rejected by the upgraded SQL guard itself.
    let mut invalid = row.clone();
    invalid["operations"][0]["requestId"] = json!("other request");
    db.execute(
        "update writing_document_checkpoints set checkpoint_json=?1",
        [invalid.to_string()],
    )
    .unwrap();
    assert!(insert_restore(&db, &Uuid::new_v4().to_string()).is_err());
    db.execute(
        "update writing_document_checkpoints set checkpoint_json=?1",
        [row.to_string()],
    )
    .unwrap();
    insert_restore(&db, &Uuid::new_v4().to_string()).unwrap();
    assert_eq!(target_row(&db), target);

    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("upgrade.sqlite3");
    db.execute("vacuum into ?1", [file.to_str().unwrap()])
        .unwrap();
    drop(db);
    let mut reopened = Connection::open(file).unwrap();
    crate::storage::migrate(&mut reopened).unwrap();
    assert_eq!(target_row(&reopened), target);
    assert_eq!(
        reopened
            .query_row("select count(*) from pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    // Compensation identity cannot be rebound or inserted twice after reopening.
    assert!(insert_restore(&reopened, &Uuid::new_v4().to_string()).is_err());
    assert!(
        reopened
            .execute(
                "update agent_editor_restorations set editor_session=?1",
                [Uuid::new_v4().to_string()]
            )
            .is_err()
    );
}

fn insert_restore(db: &Connection, owner: &str) -> rusqlite::Result<usize> {
    db.execute("insert into agent_editor_restorations(application_id,relative_path,restore_id,editor_session,expected_revision) values('op','main.tex',?1,?2,?3)", params![Uuid::new_v4().to_string(),owner,Uuid::new_v4().to_string()])
}
fn target_row(db: &Connection) -> String {
    db.query_row("select json_array(phase,target_kind,expected_editor_session,expected_editor_revision,result_editor_revision) from agent_proposal_application_files", [], |r| r.get(0)).unwrap()
}
fn guard(db: &Connection) -> String {
    db.query_row(
        "select sql from sqlite_master where name='agent_editor_restorations_admission'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
