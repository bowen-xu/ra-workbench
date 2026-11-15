use super::*;
use crate::notes::{model::NotePageDraft, page_repository::create_page_from};

fn fixture() -> (Connection, String, String, String) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let mut page = |title: &str| {
        create_page_from(
            &mut db,
            NotePageDraft {
                title: title.into(),
                parent_page_id: None,
                parent_block_position: None,
            },
        )
        .unwrap()
        .page
        .id
    };
    let source = page("source");
    let target = page("target");
    let other = page("other");
    for id in [&target, &other] {
        upsert_link_from(
            &db,
            NoteLinkDraft {
                source_page_id: source.clone(),
                source_block_id: None,
                target: crate::notes::model::NoteLinkTarget {
                    kind: "note_page".into(),
                    id: id.clone(),
                    label_snapshot: "snapshot".into(),
                    quote_snapshot: None,
                },
                relation: "reference".into(),
            },
        )
        .unwrap();
    }
    (db, source, target, other)
}

#[test]
fn backlink_read_repairs_only_orphan_sources_for_its_target() {
    let (db, source, target, other) = fixture();
    // Simulate an old/imported orphan row. Production FK cascades already
    // remove relationships when a source is deleted normally.
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute(
        "update note_links set source_page_id='missing' where source_page_id=?1",
        [&source],
    )
    .unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    assert!(
        list_backlinks_from(&db, "note_page", &target)
            .unwrap()
            .is_empty()
    );
    let rows: i64 = db
        .query_row(
            "select count(*) from note_links where target_id=?1",
            [&other],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
    assert!(
        list_backlinks_from(&db, "note_page", &other)
            .unwrap()
            .is_empty()
    );
    assert!(
        list_backlinks_from(&db, "note_page", &target)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn backlink_read_preserves_inactive_sources_and_missing_destinations() {
    let (db, source, target, _) = fixture();
    for status in ["archived", "trashed"] {
        db.execute(
            "update note_pages set status=?2 where id=?1",
            params![source, status],
        )
        .unwrap();
        assert_eq!(
            list_backlinks_from(&db, "note_page", &target)
                .unwrap()
                .len(),
            1
        );
    }
    db.execute("delete from note_pages where id=?1", [&target])
        .unwrap();
    let links = list_backlinks_from(&db, "note_page", &target).unwrap();
    assert_eq!(links.len(), 1);
    assert!(links[0].is_broken);
    assert_eq!(list_outgoing_from(&db, &source).unwrap().len(), 2);
}

#[test]
fn failed_backlink_query_does_not_clear_relations() {
    let (db, _, target, _) = fixture();
    db.execute_batch("create trigger deny_backlink_cleanup before delete on note_links begin select raise(abort,'TEST_CLEANUP_DENIED'); end;").unwrap();
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute("update note_links set source_page_id='missing'", [])
        .unwrap();
    assert!(list_backlinks_from(&db, "note_page", &target).is_err());
    let count: i64 = db
        .query_row("select count(*) from note_links", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}
