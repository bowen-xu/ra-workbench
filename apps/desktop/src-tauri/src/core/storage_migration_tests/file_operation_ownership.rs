use super::*;

#[test]
fn file_operation_upgrade_preserves_rows_and_only_unadmitted_proposals_follow_chat() {
    let mut db = super::agent_change_sets::fixture();
    super::agent_change_sets::add_file(&db, "main.tex");
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('op','set','apply')", []).unwrap();
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','main.tex','disk')", []).unwrap();
    db.execute("delete from agent_conversations where id='c'", [])
        .unwrap();
    for table in ["agent_conversations", "agent_submissions"] {
        assert_eq!(count(&db, table), 0);
    }
    for table in [
        "agent_change_sets",
        "agent_change_set_files",
        "agent_proposal_applications",
        "agent_proposal_application_files",
    ] {
        assert_eq!(count(&db, table), 1);
    }
    assert_eq!(count(&db, "content_object_gc_candidates"), 0);
    assert_eq!(
        db.prepare("pragma foreign_key_check")
            .unwrap()
            .query_map([], |_| Ok(()))
            .unwrap()
            .count(),
        0
    );
    assert!(db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('new','set','apply')", []).is_err());
    assert!(
        db.execute("update agent_change_sets set baseline_json='{}'", [])
            .is_err()
    );
    assert!(
        db.execute("update agent_change_sets set conversation_id='other'", [])
            .is_err()
    );
    db.execute(
        "insert into agent_force_delete_markers(conversation_id,request_id) values('c','deletion')",
        [],
    )
    .unwrap();
    for sql in [
        "delete from agent_proposal_application_files",
        "delete from agent_proposal_applications",
    ] {
        assert!(db.execute(sql, []).is_err());
    }

    let mut fresh = super::agent_change_sets::fixture();
    super::agent_change_sets::add_file(&fresh, "main.tex");
    crate::storage::migrate(&mut fresh).unwrap();
    fresh
        .execute("delete from agent_conversations", [])
        .unwrap();
    assert_eq!(count(&fresh, "agent_change_sets"), 0);
    assert_eq!(count(&fresh, "content_object_gc_candidates"), 2);
    assert!(fresh.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version) values('forged','c','r','t','s','m','l',?1,?1,1)", ["a".repeat(64)]).is_err());
}

#[test]
fn table_rebuild_retains_existing_admission_and_is_transactional() {
    let mut db = super::agent_change_sets::fixture();
    super::agent_change_sets::add_file(&db, "main.tex");
    for &(version, sql) in crate::storage_migrations::MIGRATIONS {
        if version <= 80 || version >= 119 {
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
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('op','set','apply')", []).unwrap();
    db.execute("insert into agent_proposal_application_files(application_id,change_set_id,relative_path,target_kind) values('op','set','main.tex','disk')", []).unwrap();
    let before: String = db.query_row("select json_array(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version,status,created_at,baseline_json) from agent_change_sets", [], |r| r.get(0)).unwrap();
    let sql = crate::storage_migrations::MIGRATIONS
        .iter()
        .find(|(v, _)| *v == 119)
        .unwrap()
        .1;
    // Rebuild the table inside a transaction and roll it back. Existing children,
    // frozen facts and protections must remain untouched even on failed upgrade.
    db.execute_batch("pragma foreign_keys=off").unwrap();
    let tx = db.transaction().unwrap();
    tx.execute_batch(sql).unwrap();
    assert_eq!(count(&tx, "agent_proposal_application_files"), 1);
    assert_eq!(count(&tx, "content_object_gc_candidates"), 0);
    tx.rollback().unwrap();
    db.execute_batch("pragma foreign_keys=on").unwrap();
    let after: String = db.query_row("select json_array(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version,status,created_at,baseline_json) from agent_change_sets", [], |r| r.get(0)).unwrap();
    assert_eq!(after, before);
    db.execute_batch("create trigger fail_file_owner_upgrade before insert on schema_migrations when new.version=119 begin select raise(abort,'fixture failure'); end").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row("pragma foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select max(version) from schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        118
    );
    assert_eq!(
        db.query_row(
            "select count(*) from migration_checksums where version=119",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(count(&db, "agent_proposal_application_files"), 1);
    db.execute_batch("drop trigger fail_file_owner_upgrade")
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        db.query_row("select baseline_json from agent_change_sets", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "null"
    );
    db.execute("delete from agent_conversations", []).unwrap();
    assert_eq!(count(&db, "agent_proposal_applications"), 1);
    assert_eq!(count(&db, "agent_proposal_application_files"), 1);
}

fn count(db: &Connection, table: &str) -> i64 {
    // Only static test-owned table names.
    db.query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
        .unwrap()
}
