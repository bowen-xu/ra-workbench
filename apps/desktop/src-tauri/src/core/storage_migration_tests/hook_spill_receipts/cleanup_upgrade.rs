//! Preserve both deployed v147 histories; install cleanup only in forward v148.
use super::*;
use sha2::{Digest, Sha256};

const DEPLOYED: &str = include_str!("../fixtures/0147_deployed.sql");
const CURRENT: &str = include_str!("../../../../migrations/0147_agent_hook_spill_receipts.sql");
const DEPLOYED_HASH: &str = "a0b376c0cd3b74b92b35e20563515b151e1376b909fb66f16859c066881ccd0d";
const CURRENT_HASH: &str = "bd009123cb810d7d3bba6365186da07df4652aed80a3a7a29dc7c31a2aeb7077";

fn deployed_fixture(path: &std::path::Path, early: bool) -> Connection {
    let db = fixture(path);
    let sql = if early { DEPLOYED } else { CURRENT };
    db.execute_batch(sql).unwrap();
    db.execute(
        "insert into schema_migrations(version,applied_at) values(147,'now')",
        [],
    )
    .unwrap();
    crate::storage_integrity::record_migration_checksum(&db, 147, sql).unwrap();
    db.execute_batch("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
        values('bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','bb/bb/evidence','text/plain',8,'now');
        insert into agent_hook_spill_artifacts(id,source_conversation_id,source_window_id,turn_id,phase,model_json,content_sha256)
        values('00000000-0000-4000-8000-000000000011','00000000-0000-4000-8000-000000000012','00000000-0000-4000-8000-000000000013','00000000-0000-4000-8000-000000000014','manual','{}','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb');
        insert into agent_hook_spill_previews(artifact_id,preview_sha256)
        values('00000000-0000-4000-8000-000000000011','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc');").unwrap();
    db
}
fn checksum(db: &Connection) -> String {
    db.query_row(
        "select sha256 from migration_checksums where version=147",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn triggers(db: &Connection) -> Vec<(String, String)> {
    db.prepare("select name,sql from sqlite_master where type='trigger' and name in ('agent_hook_spill_origin_delete','agent_hook_spill_grant_delete') order by name")
        .unwrap().query_map([], |r| Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
}
fn preview(db: &Connection) -> String {
    db.query_row(
        "select preview_sha256 from agent_hook_spill_previews",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn hook_spill_cleanup_freezes_both_known_147_definitions() {
    assert_eq!(
        format!("{:x}", Sha256::digest(DEPLOYED.as_bytes())),
        DEPLOYED_HASH
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(CURRENT.as_bytes())),
        CURRENT_HASH
    );
}
#[test]
fn hook_spill_cleanup_upgrades_both_deployed_schemas_without_rewriting_evidence_or_hashes() {
    for early in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("upgrade.sqlite");
        let mut db = deployed_fixture(&path, early);
        let before = references(&db);
        let old_hashes = hashes(&db);
        let hash = checksum(&db);
        let text = preview(&db);
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(triggers(&db).len(), 2);
        assert_eq!(references(&db), before);
        assert_eq!(hashes(&db), old_hashes);
        assert_eq!(checksum(&db), hash);
        assert_eq!(preview(&db), text);
        assert_eq!(
            db.query_row("select count(*) from pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        drop(db);
        let mut db = Connection::open(path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(checksum(&db), hash);
        assert_eq!(references(&db), before);
        assert_eq!(preview(&db), text);
        assert_eq!(triggers(&db).len(), 2);
    }
}
#[test]
fn hook_spill_cleanup_failure_rolls_back_trigger_replacement_and_can_retry() {
    for early in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let mut db = deployed_fixture(&root.path().join("fault.sqlite"), early);
        let old = triggers(&db);
        let before = references(&db);
        let hash = checksum(&db);
        db.execute_batch("create trigger reject_148 before insert on schema_migrations when new.version=148 begin select raise(abort,'fault'); end;").unwrap();
        assert!(crate::storage::migrate(&mut db).is_err());
        assert_eq!(triggers(&db), old);
        assert_eq!(references(&db), before);
        assert_eq!(checksum(&db), hash);
        assert_eq!(
            db.query_row(
                "select count(*) from migration_checksums where version=148",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger reject_148").unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(triggers(&db).len(), 2);
        assert_eq!(references(&db), before);
        assert_eq!(checksum(&db), hash);
    }
}
#[test]
fn hook_spill_cleanup_rejects_unknown_hash_before_forward_migration() {
    let root = tempfile::tempdir().unwrap();
    let mut db = deployed_fixture(&root.path().join("unknown.sqlite"), true);
    db.execute(
        "update migration_checksums set sha256=?1 where version=147",
        ["d".repeat(64)],
    )
    .unwrap();
    let before = references(&db);
    let error = crate::storage::migrate(&mut db).unwrap_err().to_string();
    assert!(error.contains("迁移 147"));
    assert!(error.contains(&format!("数据库记录：{}", "d".repeat(64))));
    assert!(error.contains(&format!("当前构建：{CURRENT_HASH}")));
    assert!(triggers(&db).is_empty());
    assert_eq!(references(&db), before);
    assert_eq!(checksum(&db), "d".repeat(64));
    assert_eq!(
        db.query_row(
            "select count(*) from schema_migrations where version=148",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn hook_spill_cleanup_compatibility_does_not_accept_further_edits_to_147() {
    let root = tempfile::tempdir().unwrap();
    let db = deployed_fixture(&root.path().join("changed.sqlite"), true);
    crate::storage_integrity::verify_recorded_migration_checksum(&db, 147, CURRENT).unwrap();
    assert!(
        crate::storage_integrity::verify_recorded_migration_checksum(
            &db,
            147,
            &format!("{CURRENT}\n")
        )
        .is_err()
    );
    assert_eq!(checksum(&db), DEPLOYED_HASH);
}
