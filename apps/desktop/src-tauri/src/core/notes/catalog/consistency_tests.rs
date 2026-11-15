use super::*;
use crate::notes::{
    document_repository::{load_document_from, save_document_from},
    model::{NoteDocumentBlock, NoteDocumentSaveRequest},
};
use rusqlite::StatementStatus;
use serde_json::json;

#[test]
fn catalog_rich_text_title_matches_the_existing_frontend_projection() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    let mut current = load_document_from(&db, &id).unwrap();
    current.document.blocks[0].content = json!({"kind":"inline","items":[
        {"type":"text","text":"核心","styles":["bold"]},
        {"type":"text","text":"概念","styles":[]}
    ]});
    save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document: current.document,
            expected_revision: current.page.revision,
            save_reason: "manual".to_owned(),
        },
    )
    .unwrap();
    let page = query_from(&db, super::tests::query()).unwrap();
    assert_eq!(
        page.items[0].title, "核心概念",
        "style boundaries cannot insert spaces in the cached display title"
    );
}

#[test]
fn catalog_metadata_only_update_does_not_revisit_a_large_reading_body() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    let mut current = load_document_from(&db, &id).unwrap();
    current.document.blocks.extend((0..1000).map(|index|NoteDocumentBlock{
        id:format!("canonical-{index}"),block_type:"paragraph".to_owned(),props:json!({}),
        content:json!({"kind":"inline","items":[{"type":"text","text":"unchanged body","styles":[]}]}),children:vec![],
    }));
    save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document: current.document,
            expected_revision: current.page.revision,
            save_reason: "manual".to_owned(),
        },
    )
    .unwrap();
    let mut update = db
        .prepare("update note_pages set is_favorite=1,revision=revision+1 where id=?1")
        .unwrap();
    update.execute([&id]).unwrap();
    assert!(
        update.get_status(StatementStatus::VmStep) < 5000,
        "metadata-only writes must reuse the persisted body-title projection"
    );
}

#[test]
fn catalog_title_refresh_waits_for_the_canonical_page_revision_boundary() {
    let mut connection = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&connection);
    let id = super::tests::reading(&mut connection, "highlight", "quote");
    let initial: i64 = connection
        .query_row(
            "select generation from note_page_catalog_state where id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let tx = connection.transaction().unwrap();
    for index in 0..1000 {
        tx.execute("insert into note_blocks(id,scope_kind,page_id,position,block_type,content_json,plain_text,created_at,updated_at) values(?1,'page',?2,?3,'paragraph','{\"kind\":\"none\"}','body','now','now')",rusqlite::params![format!("extra-{index}"),id,index+10]).unwrap();
    }
    tx.execute(
        "update note_blocks set plain_text='已保存标题',content_json=?2 where page_id=?1 and position=0",
        rusqlite::params![&id,serde_json::json!({"kind":"inline","items":[{"type":"text","text":"已保存标题","styles":[]}]}).to_string()],
    )
    .unwrap();
    let before_revision: i64 = tx
        .query_row(
            "select generation from note_page_catalog_state where id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        before_revision, initial,
        "block writes must not refresh the whole reading tree"
    );
    let mut advance = tx
        .prepare("update note_pages set revision=revision+1 where id=?1")
        .unwrap();
    advance.execute([&id]).unwrap();
    assert!(
        advance.get_status(StatementStatus::VmStep) < 200_000,
        "title traversal must seek each parent rather than scan the page for each recursive row"
    );
    drop(advance);
    tx.commit().unwrap();
    let after = query_from(&connection, super::tests::query()).unwrap();
    assert_eq!(after.items[0].title, "已保存标题");
    // The public source revision also includes text changes. This assertion
    // counts header refreshes only, just as the pre-commit checks above do.
    let after_headers: i64 = connection
        .query_row(
            "select generation from note_page_catalog_state where id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        after_headers - initial <= 2,
        "one canonical commit must refresh the title once"
    );
}
