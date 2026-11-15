use super::conflict_resolution::*;
use super::{model::NotePageDraft, page_repository::create_page_from, tests::migrated_connection};
use rusqlite::Connection;
use serde_json::json;
use uuid::Uuid;

fn fixture() -> (Connection, Request) {
    let mut db = migrated_connection();
    let source = create_page_from(
        &mut db,
        NotePageDraft {
            title: "Synthetic note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .unwrap();
    db.execute("insert into agent_note_patch_candidates(conversation_id,page_id,proposal_id,generation,baseline_json,candidate_json,state) values('conversation',?1,'proposal',1,'{}','{}','pending')", [&source.page.id]).unwrap();
    let request: Request = serde_json::from_value(json!({ "requestId": Uuid::new_v4(), "document": source.document, "expectedRevision": source.page.revision, "copy": true })).unwrap();
    (db, request)
}
fn count(db: &Connection, table: &str) -> i64 {
    // Only static table names from these tests, never external input.
    db.query_row(&format!("select count(*) from {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
#[test]
fn conflict_resolution_copies_retires_and_replays_exact_receipt() {
    let (mut db, request) = fixture();
    let input = serde_json::to_value(&request).unwrap();
    let first = resolve(&mut db, request).unwrap();
    let replay = resolve(&mut db, serde_json::from_value(input.clone()).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    assert_eq!(count(&db, "note_pages"), 2);
    assert_eq!(count(&db, "note_conflict_resolutions"), 1);
    assert_eq!(
        db.query_row("select state from agent_note_patch_candidates", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "rejected"
    );
    let mut changed = input;
    changed["copy"] = json!(false);
    assert!(resolve(&mut db, serde_json::from_value(changed).unwrap()).is_err());
    assert_eq!(count(&db, "note_pages"), 2);
}
#[test]
fn conflict_resolution_rolls_back_copy_body_history_and_review_on_receipt_failure() {
    let (mut db, request) = fixture();
    let id = request.document.page_id.clone();
    db.execute_batch("create trigger fail_conflict_receipt before insert on note_conflict_resolutions begin select raise(abort,'synthetic disk failure'); end;").unwrap();
    assert!(resolve(&mut db, request).is_err());
    assert_eq!(count(&db, "note_pages"), 1);
    assert_eq!(count(&db, "note_revisions"), 0);
    assert_eq!(
        db.query_row("select revision from note_pages where id=?1", [id], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select state from agent_note_patch_candidates", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "pending"
    );
}
#[test]
fn conflict_resolution_rejects_stale_version_before_creating_copy() {
    let (mut db, request) = fixture();
    db.execute("update note_pages set revision=1", []).unwrap();
    assert!(resolve(&mut db, request).is_err());
    assert_eq!(count(&db, "note_pages"), 1);
    assert_eq!(count(&db, "note_conflict_resolutions"), 0);
}
#[test]
fn conflict_resolution_upgrade_and_reopen_preserve_existing_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("notes.sqlite");
    let mut db = Connection::open(&file).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 183);
    crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
    let checksums = db
        .prepare("select version,sha256 from migration_checksums order by version")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let source = create_page_from(
        &mut db,
        NotePageDraft {
            title: "Existing synthetic note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .unwrap();
    let document = serde_json::to_string(&source.document).unwrap();
    db.execute("insert into agent_note_patch_candidates(conversation_id,page_id,proposal_id,generation,baseline_json,candidate_json,state) values('conversation',?1,'proposal',7,?2,?2,'pending')", rusqlite::params![source.page.id, document]).unwrap();
    let candidate = |db: &Connection| {
        db.query_row("select page_id,generation,baseline_json,candidate_json,state from agent_note_patch_candidates where conversation_id='conversation' and proposal_id='proposal'", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?))).unwrap()
    };
    let before = candidate(&db);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(candidate(&db), before);
    assert_eq!(count(&db, "note_conflict_resolutions"), 0);
    drop(db);
    let mut reopened = Connection::open(file).unwrap();
    crate::storage::migrate(&mut reopened).unwrap();
    assert_eq!(candidate(&reopened), before);
    assert_eq!(
        serde_json::to_value(
            super::document_repository::load_document_from(&reopened, &source.page.id)
                .unwrap()
                .document
        )
        .unwrap(),
        serde_json::to_value(source.document).unwrap()
    );
    assert_eq!(count(&reopened, "note_conflict_resolutions"), 0);
    for (version, checksum) in checksums {
        assert_eq!(
            reopened
                .query_row(
                    "select sha256 from migration_checksums where version=?1",
                    [version],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            checksum
        );
    }
    assert!(
        reopened
            .prepare("pragma foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}
