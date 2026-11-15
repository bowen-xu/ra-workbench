use super::*;
use std::path::Path;
fn fixture(db: &mut Connection) -> Uuid {
    crate::storage_migration_tests::migrate_through(db, 78);
    db.execute_batch("pragma foreign_keys=on").unwrap();
    let id = Uuid::new_v4();
    db.execute("insert into research_projects(id,title,summary,created_at) values ('project','fixture','','now')", []).unwrap();
    db.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values (?1,'project','fixture','main.tex','now','now')", [id.to_string()]).unwrap();
    id
}
fn identity() -> InstructionRootIdentity<'static> {
    InstructionRootIdentity {
        canonical_path: Path::new("/synthetic/workspace"),
        device: u64::MAX,
        inode: 42,
    }
}
#[test]
fn persisted_grant_matches_identity_and_revocation_prevents_stale_update() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("trust.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let id = fixture(&mut db);
    let root = identity();
    assert!(!is_trusted(&db, id, &root).unwrap());
    let grant = update(&mut db, id, 0, &root, true).unwrap();
    assert_eq!(grant.revision, 1);
    drop(db);
    let mut db = Connection::open(path).unwrap();
    assert!(is_trusted(&db, id, &root).unwrap());
    assert!(
        !is_trusted(
            &db,
            id,
            &InstructionRootIdentity {
                inode: 43,
                ..identity()
            }
        )
        .unwrap()
    );
    assert!(
        !is_trusted(
            &db,
            id,
            &InstructionRootIdentity {
                canonical_path: Path::new("/elsewhere"),
                ..identity()
            }
        )
        .unwrap()
    );
    assert_eq!(
        update(&mut db, id, 0, &root, false),
        Err(TrustError::Conflict)
    );
    assert_eq!(update(&mut db, id, 1, &root, false).unwrap().revision, 2);
    assert!(!is_trusted(&db, id, &root).unwrap());
    assert_eq!(
        update(&mut db, id, 1, &root, true),
        Err(TrustError::Conflict)
    );
}
#[test]
fn inactive_or_unknown_manuscripts_cannot_grant_and_delete_cascades() {
    let mut db = Connection::open_in_memory().unwrap();
    let id = fixture(&mut db);
    assert_eq!(
        update(&mut db, Uuid::new_v4(), 0, &identity(), true),
        Err(TrustError::Inactive)
    );
    update(&mut db, id, 0, &identity(), true).unwrap();
    db.execute(
        "update manuscripts set status='archived' where id=?1",
        [id.to_string()],
    )
    .unwrap();
    assert_eq!(is_trusted(&db, id, &identity()), Err(TrustError::Inactive));
    assert_eq!(
        update(&mut db, id, 1, &identity(), true),
        Err(TrustError::Inactive)
    );
    db.execute("delete from manuscripts where id=?1", [id.to_string()])
        .unwrap();
    let count: i64 = db
        .query_row(
            "select count(*) from agent_manuscript_instruction_trust",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
