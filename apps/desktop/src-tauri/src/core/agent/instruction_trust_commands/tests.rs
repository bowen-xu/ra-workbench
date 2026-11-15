use super::*;
use rusqlite::Connection;
fn fixture() -> (tempfile::TempDir, AppStore, Uuid) {
    let temp = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 78);
    let id = Uuid::new_v4();
    db.execute("insert into research_projects(id,title,summary,created_at) values ('project','fixture','','now')", []).unwrap();
    db.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values (?1,'project','fixture','main.tex','now','now')", [id.to_string()]).unwrap();
    let store = AppStore::for_test(db, temp.path().join("data/objects"));
    std::fs::create_dir_all(
        store
            .manuscript_directory()
            .join(id.to_string())
            .join("source"),
    )
    .unwrap();
    (temp, store, id)
}
#[test]
fn wire_rejects_paths_and_exposes_no_native_identity() {
    let id = Uuid::new_v4();
    assert!(serde_json::from_value::<UpdateRequest>(serde_json::json!({"manuscriptId":id,"expectedRevision":0,"granted":true,"canonicalPath":"/outside"})).is_err());
    assert!(
        serde_json::from_value::<ReadRequest>(serde_json::json!({"manuscriptId":"invalid"}))
            .is_err()
    );
    let response = TrustResponse {
        schema_version: 1,
        manuscript_id: id,
        revision: 1,
        granted: true,
        root_matches: true,
    };
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert_eq!(value["rootMatches"], true);
}
#[cfg(unix)]
#[test]
fn native_settings_grant_checks_root_and_revocation_works_after_root_disappears() {
    let (_temp, store, id) = fixture();
    let before = read(&store, id).unwrap();
    assert_eq!(before.revision, 0);
    assert!(!before.granted);
    let granted = update(
        &store,
        UpdateRequest {
            manuscript_id: id,
            expected_revision: 0,
            granted: true,
        },
    )
    .unwrap();
    assert!(granted.root_matches);
    assert!(read(&store, id).unwrap().root_matches);
    let root = root(&store, id).unwrap();
    std::fs::rename(&root, root.with_file_name("old-source")).unwrap();
    let missing = read(&store, id).unwrap();
    assert!(missing.granted);
    assert!(!missing.root_matches);
    assert!(matches!(
        update(
            &store,
            UpdateRequest {
                manuscript_id: id,
                expected_revision: 0,
                granted: false
            }
        ),
        Err(TrustCommandError::Conflict)
    ));
    let revoked = update(
        &store,
        UpdateRequest {
            manuscript_id: id,
            expected_revision: 1,
            granted: false,
        },
    )
    .unwrap();
    assert!(!revoked.granted);
    assert_eq!(read(&store, id).unwrap().revision, 2);
}
