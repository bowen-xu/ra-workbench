use super::*;

#[test]
fn deployed_228_proposals_keep_journals_and_deduplicate_per_lease_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade-228.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 228);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now');
        insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now');
        insert into agent_conversations(id,title) values('c','c');
        insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values('c','r','t','s',1,'{}');").unwrap();
    let hash = "a".repeat(64);
    let object = "b".repeat(64);
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,?1,'text/plain',0,'now')", [&object]).unwrap();
    db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version,baseline_json) values('old','c','r','t','s','m','lease-1',?1,?1,1,'{}')", [&hash]).unwrap();
    db.execute("insert into agent_change_set_files(change_set_id,relative_path,after_sha256,after_executable) values('old','main.tex',?1,0)", [&object]).unwrap();
    db.execute("insert into agent_proposal_applications(id,change_set_id,direction) values('application','old','apply')", []).unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=228",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_229 before insert on schema_migrations when new.version=229 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from agent_change_sets where id='old'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert!(db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version) values('rollback','c','r','t','s','m','lease-2',?1,?1,1)", [&hash]).is_err());
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_229;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state: (String,String,String,u64,u64,String) = db.query_row("select (select lease_id from agent_change_sets where id='old'),(select after_sha256 from agent_change_set_files where change_set_id='old'),(select change_set_id from agent_proposal_applications where id='application'),(select count(*) from pragma_foreign_key_check),(select count(*) from pragma_foreign_key_list('agent_change_set_files') where \"table\"='agent_change_sets'),(select sha256 from migration_checksums where version=228)", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        state,
        ("lease-1".into(), object, "old".into(), 0, 1, checksum)
    );
    db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version) values('new','c','r','t','s','m','lease-2',?1,?1,1)", [&hash]).unwrap();
    assert!(db.execute("insert into agent_change_sets(id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,inventory_hash,schema_version) values('duplicate','c','r','t','s','m','lease-2',?1,?1,1)", [&hash]).is_err());
    assert!(
        db.execute(
            "update agent_change_sets set lease_id='changed' where id='old'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            "update agent_change_sets set baseline_json='null' where id='old'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute("delete from agent_change_sets where id='old'", [])
            .is_err()
    );
}

#[test]
fn deployed_223_delivery_proof_migration_rolls_back_and_reopens() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 223);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values('legacy','Preserved')",
        [],
    )
    .unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=223",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_224 before insert on schema_migrations when new.version=224 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='writing_agent_delivery_proofs'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_224;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state: (String, u64, String, u64) = db.query_row("select (select title from agent_conversations where id='legacy'),(select count(*) from writing_agent_delivery_proofs),(select sha256 from migration_checksums where version=223),(select count(*) from pragma_foreign_key_check)", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
    assert_eq!(state, ("Preserved".into(), 0, checksum, 0));
}

#[test]
fn deployed_224_upgrade_preserves_unlinked_proof_and_reopens() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade-224.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 224);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now'); insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now');").unwrap();
    let old_id = "00000000-0000-4000-8000-000000000001";
    db.execute("insert into writing_agent_delivery_proofs(delivery_id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,proof_json) values(?1,?1,?1,?1,?1,'m',?1,?2,'{\"schemaVersion\":1,\"kind\":\"empty\"}')", rusqlite::params![old_id, "a".repeat(64)]).unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=224",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_225 before insert on schema_migrations when new.version=225 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='writing_agent_delivery_tool_calls'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_225;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state: (String, u64, String, u64) = db.query_row("select (select proof_json from writing_agent_delivery_proofs where delivery_id=?1),(select count(*) from writing_agent_delivery_tool_calls),(select sha256 from migration_checksums where version=224),(select count(*) from pragma_foreign_key_check)", [old_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        state,
        (
            "{\"schemaVersion\":1,\"kind\":\"empty\"}".into(),
            0,
            checksum,
            0
        )
    );
    db.execute_batch("pragma foreign_keys=on;").unwrap();
    db.execute("insert into writing_agent_delivery_tool_calls(delivery_id,item_id,call_id) values(?1,?1,?1)", [old_id]).unwrap();
    assert!(
        db.execute(
            "update writing_agent_delivery_tool_calls set item_id=?1",
            [old_id]
        )
        .is_err()
    );
    assert!(
        db.execute("delete from writing_agent_delivery_tool_calls", [])
            .is_err()
    );
    db.execute("delete from manuscripts where id='m'", [])
        .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from writing_agent_delivery_tool_calls",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn deployed_225_result_migration_rolls_back_and_reopens() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade-225.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 225);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values('legacy','Preserved')",
        [],
    )
    .unwrap();
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now'); insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now');").unwrap();
    let old_id = "00000000-0000-4000-8000-000000000001";
    db.execute("insert into writing_agent_delivery_proofs(delivery_id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,proof_json) values(?1,?1,?1,?1,?1,'m',?1,?2,'{\"schemaVersion\":1,\"kind\":\"empty\"}')", rusqlite::params![old_id, "a".repeat(64)]).unwrap();
    db.execute("insert into writing_agent_delivery_tool_calls(delivery_id,item_id,call_id) values(?1,?1,?1)", [old_id]).unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=225",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_226 before insert on schema_migrations when new.version=226 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='writing_agent_delivery_results'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_226;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state: (String, u64, String, u64) = db.query_row("select (select title from agent_conversations where id='legacy'),(select count(*) from writing_agent_delivery_results),(select sha256 from migration_checksums where version=225),(select count(*) from pragma_foreign_key_check)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(state, ("Preserved".into(), 0, checksum, 0));
    assert_eq!(
        db.query_row(
            "select count(*) from writing_agent_delivery_tool_calls where delivery_id=?1",
            [old_id],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    db.execute_batch("pragma foreign_keys=on;").unwrap();
    db.execute(
        "insert into writing_agent_delivery_results(delivery_id,result_item_id) values(?1,?1)",
        [old_id],
    )
    .unwrap();
    assert!(
        db.execute(
            "update writing_agent_delivery_results set result_item_id=?1",
            [old_id]
        )
        .is_err()
    );
    assert!(
        db.execute("delete from writing_agent_delivery_results", [])
            .is_err()
    );
    db.execute("delete from manuscripts where id='m'", [])
        .unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from writing_agent_delivery_results",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn deployed_226_workspace_generation_migration_rolls_back_and_reopens() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade-226.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 226);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute(
        "insert into agent_conversations(id,title) values('legacy','Preserved')",
        [],
    )
    .unwrap();
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=226",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_227 before insert on schema_migrations when new.version=227 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    assert_eq!(
        db.query_row(
            "select count(*) from sqlite_schema where name='writing_agent_workspace_generations'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_227;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let state: (String,u64,String,u64) = db.query_row("select (select title from agent_conversations where id='legacy'),(select count(*) from writing_agent_workspace_generations),(select sha256 from migration_checksums where version=226),(select count(*) from pragma_foreign_key_check)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(state, ("Preserved".into(), 0, checksum, 0));
}

#[test]
fn deployed_227_generation_guard_preserves_intent_and_allows_manuscript_deletion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade-227.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    migrate_through(&mut db, 227);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('p','p','','now'); insert into manuscripts(id,project_id,title,created_at,updated_at) values('m','p','m','now','now'); insert into agent_conversations(id,title) values('c','c'); insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values('c','r','t','s',1,'{}');").unwrap();
    let id = "00000000-0000-4000-8000-000000000001";
    db.execute("insert into writing_agent_delivery_proofs(delivery_id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,proof_json) values(?1,?1,?1,?1,?1,'m',?1,?2,'{}')", rusqlite::params![id,"a".repeat(64)]).unwrap();
    db.execute("insert into writing_agent_delivery_tool_calls(delivery_id,item_id,call_id) values(?1,?1,?1)", [id]).unwrap();
    db.execute(
        "insert into writing_agent_delivery_results(delivery_id,result_item_id) values(?1,?1)",
        [id],
    )
    .unwrap();
    db.execute("insert into writing_agent_workspace_generations(conversation_id,request_id,generation,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,manifest_json,predecessor_delivery_id) values('c','r',1,'t','s','m','lease',?1,'{}',?2)", rusqlite::params!["b".repeat(64),id]).unwrap();
    assert!(
        db.execute("delete from manuscripts where id='m'", [])
            .is_err()
    );
    let checksum: String = db
        .query_row(
            "select sha256 from migration_checksums where version=227",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("create trigger reject_228 before insert on schema_migrations when new.version=228 begin select raise(abort,'synthetic migration failure'); end;").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("drop trigger reject_228;").unwrap();
    crate::storage::migrate(&mut db).unwrap();
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute_batch("pragma foreign_keys=on;").unwrap();
    assert_eq!(
        db.query_row(
            "select sha256 from migration_checksums where version=227",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        checksum
    );
    assert!(
        db.execute("delete from writing_agent_workspace_generations", [])
            .is_err()
    );
    db.execute("delete from manuscripts where id='m'", [])
        .unwrap();
    let remaining: (u64,u64,u64) = db.query_row("select (select count(*) from writing_agent_workspace_generations),(select count(*) from writing_agent_delivery_proofs),(select count(*) from pragma_foreign_key_check)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(remaining, (0, 0, 0));
}
