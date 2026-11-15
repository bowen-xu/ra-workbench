use super::{
    link_repository::{list_backlinks_from, list_outgoing_from},
    literature_notes,
    model::{NotePageMetadataUpdate, ReadingNoteDraft},
    page_repository::update_page_from,
    reading_note_repository::{create_reading_note_from, reading_note_from},
    tests::insert_paper_fixture,
};
use rusqlite::Connection;

fn draft(id: &str, note_type: &str, quote: &str) -> ReadingNoteDraft {
    ReadingNoteDraft {
        id: Some(id.to_owned()),
        source_anchor_id: None,
        note_type: note_type.to_owned(),
        body: "独立记录".to_owned(),
        color: None,
        item_id: "paper-1".to_owned(),
        attachment_id: "attachment-1".to_owned(),
        page_index: 0,
        page_label: None,
        rects: vec![],
        quote_exact: quote.to_owned(),
        quote_prefix: None,
        quote_suffix: None,
        selector: None,
    }
}

#[test]
fn note_index_origin_links_upgrade_and_reopen() {
    let path = std::env::temp_dir().join(format!(
        "note-index-upgrade-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let mut connection = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut connection, 187);
    insert_paper_fixture(&connection);
    let sticky =
        create_reading_note_from(&mut connection, draft("sticky", "page_note", "")).unwrap();
    assert!(
        list_outgoing_from(&connection, &sticky.page_id)
            .unwrap()
            .is_empty()
    );
    crate::storage::migrate(&mut connection).unwrap();
    assert_eq!(
        list_outgoing_from(&connection, &sticky.page_id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        list_backlinks_from(&connection, "paper", "paper-1").unwrap()[0].source_page_id,
        sticky.page_id
    );
    create_reading_note_from(&mut connection, draft("highlight", "highlight", "摘录")).unwrap();
    assert_eq!(
        literature_notes::list_from(&connection, "paper-1")
            .unwrap()
            .len(),
        1
    );
    let favorite = update_page_from(
        &mut connection,
        NotePageMetadataUpdate {
            page_id: sticky.page_id.clone(),
            title: None,
            is_favorite: Some(true),
            project_ids: None,
            tag_ids: None,
            expected_revision: 0,
        },
    )
    .unwrap();
    assert_eq!(favorite.updated_at, sticky.updated_at);
    let highlight = reading_note_from(&connection, "highlight").unwrap();
    assert!(
        update_page_from(
            &mut connection,
            NotePageMetadataUpdate {
                page_id: highlight.page_id,
                title: None,
                is_favorite: Some(true),
                project_ids: None,
                tag_ids: None,
                expected_revision: 0
            }
        )
        .is_err()
    );
    drop(connection);
    let mut connection = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut connection).unwrap();
    assert_eq!(
        list_outgoing_from(&connection, &sticky.page_id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        reading_note_from(&connection, "sticky").unwrap().body,
        "独立记录"
    );
    connection
        .execute("delete from bibliographic_items where id='paper-1'", [])
        .unwrap();
    assert_eq!(
        reading_note_from(&connection, "sticky").unwrap().status,
        "active"
    );
    assert!(list_outgoing_from(&connection, &sticky.page_id).unwrap()[0].is_broken);
    drop(connection);
    std::fs::remove_file(path).unwrap();
}
