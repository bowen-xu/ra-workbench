use crate::{
    hbs_storage::{self, Commit},
    notes::{self, model::NotePageDraft},
};
use rusqlite::Connection;
use serde_json::{Value, json};
fn page(db: &mut Connection, title: &str, parent: Option<String>) -> String {
    notes::page_creation::create_page_from(
        db,
        NotePageDraft {
            title: title.into(),
            parent_page_id: parent,
            parent_block_position: None,
        },
    )
    .unwrap()
    .page
    .id
}
fn entry(id: &str, target: &str, owned: bool) -> Value {
    json!({"id":id,"type":"researchObjectReference","props":{"pageEntry":owned},"children":[],"content":{"kind":"reference","targetKind":"note_page","targetId":target,"labelSnapshot":"Page","quoteSnapshot":""}})
}
fn write(
    db: &mut Connection,
    id: &str,
    key: &str,
    point: &Value,
    blocks: Value,
) -> Result<Value, String> {
    let mut doc = point["saved"].clone();
    doc["blocks"] = blocks;
    crate::hbs_storage::test_sources::commit(
        db,
        "note",
        id,
        Commit {
            id: key.into(),
            expected_version: point["version"].as_i64().unwrap(),
            saved: doc.clone(),
            baseline: doc,
        },
    )
    .map(|v| v["snapshot"].clone())
}
fn state(db: &Connection, id: &str) -> String {
    db.query_row("select status from note_pages where id=?1", [id], |r| {
        r.get(0)
    })
    .unwrap()
}
#[test]
fn cut_paste_and_history_restore_one_page_without_another_review_or_undo_stack() {
    for legacy in [false, true] {
        let mut db = notes::tests::migrated_connection();
        let source = page(&mut db, "Source", None);
        let target = page(&mut db, "Child", legacy.then(|| source.clone()));
        let initial = hbs_storage::read(&db, "note", &source).unwrap();
        let with = if legacy {
            initial
        } else {
            write(
                &mut db,
                &source,
                "insert",
                &initial,
                json!([entry("owned", &target, true)]),
            )
            .unwrap()
        };
        let cut = write(&mut db, &source, "cut", &with, json!([])).unwrap();
        assert_eq!(state(&db, &target), "trashed");
        let pasted = write(
            &mut db,
            &source,
            "paste",
            &cut,
            json!([entry("pasted-identity", &target, true)]),
        )
        .unwrap();
        assert_eq!(state(&db, &target), "active");
        let undone = write(&mut db, &source, "undo-paste", &pasted, json!([])).unwrap();
        assert_eq!(state(&db, &target), "trashed");
        write(
            &mut db,
            &source,
            "redo-paste",
            &undone,
            json!([entry("pasted-identity", &target, true)]),
        )
        .unwrap();
        assert_eq!(state(&db, &target), "active");
        let count: i64 = db
            .query_row(
                "select count(*) from note_pages where id=?1",
                [&target],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
#[test]
fn pasting_into_another_page_restores_the_owned_tree_and_detaches_legacy_parentage() {
    let mut db = notes::tests::migrated_connection();
    let source = page(&mut db, "Source", None);
    let destination = page(&mut db, "Destination", None);
    let child = page(&mut db, "Child", Some(source.clone()));
    let nested = page(&mut db, "Nested", None);
    let c = hbs_storage::read(&db, "note", &child).unwrap();
    let c = write(
        &mut db,
        &child,
        "nested",
        &c,
        json!([entry("nested-entry", &nested, true)]),
    )
    .unwrap();
    let source_before = hbs_storage::read(&db, "note", &source).unwrap();
    write(&mut db, &source, "cut-tree", &source_before, json!([])).unwrap();
    assert_eq!(state(&db, &child), "trashed");
    assert_eq!(state(&db, &nested), "trashed");
    let dest = hbs_storage::read(&db, "note", &destination).unwrap();
    write(
        &mut db,
        &destination,
        "paste-tree",
        &dest,
        json!([entry("new-clipboard-id", &child, true)]),
    )
    .unwrap();
    assert_eq!(state(&db, &child), "active");
    assert_eq!(state(&db, &nested), "active");
    let parent: Option<String> = db
        .query_row(
            "select parent_page_id from note_pages where id=?1",
            [&child],
            |r| r.get(0),
        )
        .unwrap();
    assert!(parent.is_none());
    assert_eq!(
        hbs_storage::read(&db, "note", &child).unwrap()["saved"],
        c["saved"]
    );
}
#[test]
fn another_owned_entrance_retains_target_but_ordinary_links_do_not_own_lifecycle() {
    let mut db = notes::tests::migrated_connection();
    let source = page(&mut db, "Source", None);
    let other = page(&mut db, "Other", None);
    let target = page(&mut db, "Child", None);
    let s = hbs_storage::read(&db, "note", &source).unwrap();
    let s = write(
        &mut db,
        &source,
        "own",
        &s,
        json!([entry("one", &target, true)]),
    )
    .unwrap();
    let o = hbs_storage::read(&db, "note", &other).unwrap();
    let o = write(
        &mut db,
        &other,
        "share",
        &o,
        json!([entry("two", &target, true)]),
    )
    .unwrap();
    write(&mut db, &source, "cut", &s, json!([])).unwrap();
    assert_eq!(state(&db, &target), "active");
    let o = write(
        &mut db,
        &other,
        "last-cut",
        &o,
        json!([entry("ordinary", &target, false)]),
    )
    .unwrap();
    assert_eq!(state(&db, &target), "trashed");
    write(
        &mut db,
        &other,
        "paste-ordinary",
        &o,
        json!([
            entry("ordinary", &target, false),
            entry("ordinary-copy", &target, false)
        ]),
    )
    .unwrap();
    assert_eq!(state(&db, &target), "trashed");
}
#[test]
fn failed_paste_rolls_back_target_source_baseline_and_receipt_then_same_request_can_retry() {
    let mut db = notes::tests::migrated_connection();
    let source = page(&mut db, "Source", None);
    let target = page(&mut db, "Child", None);
    let s = hbs_storage::read(&db, "note", &source).unwrap();
    let s = write(
        &mut db,
        &source,
        "own",
        &s,
        json!([entry("owned", &target, true)]),
    )
    .unwrap();
    let cut = write(&mut db, &source, "cut", &s, json!([])).unwrap();
    db.execute_batch("create trigger fail_paste_baseline before update on hbs_note_baselines begin select raise(abort,'synthetic paste failure');end;").unwrap();
    assert!(
        write(
            &mut db,
            &source,
            "paste",
            &cut,
            json!([entry("pasted", &target, true)])
        )
        .unwrap_err()
        .contains("synthetic paste failure")
    );
    assert_eq!(state(&db, &target), "trashed");
    assert_eq!(hbs_storage::read(&db, "note", &source).unwrap(), cut);
    db.execute_batch("drop trigger fail_paste_baseline")
        .unwrap();
    let pasted = write(
        &mut db,
        &source,
        "paste",
        &cut,
        json!([entry("pasted", &target, true)]),
    )
    .unwrap();
    let revision: i64 = db
        .query_row(
            "select revision from note_pages where id=?1",
            [&target],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        write(
            &mut db,
            &source,
            "paste",
            &cut,
            json!([entry("pasted", &target, true)])
        )
        .unwrap(),
        pasted
    );
    assert_eq!(
        db.query_row(
            "select revision from note_pages where id=?1",
            [&target],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        revision
    );
}
