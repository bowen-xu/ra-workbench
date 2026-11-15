use super::*;
use crate::notes::{
    document_repository::{load_document_from, save_document_from},
    model::{NoteDocument, NoteDocumentBlock, NoteDocumentSaveRequest},
};
use serde_json::{Value, json};
use std::collections::HashMap;

fn save(db: &mut Connection, id: &str, mut document: NoteDocument) {
    let current = load_document_from(db, id).unwrap();
    document.page_id = id.to_owned();
    save_document_from(
        db,
        NoteDocumentSaveRequest {
            document,
            expected_revision: current.page.revision,
            save_reason: "manual".to_owned(),
        },
    )
    .unwrap();
}
fn body_count(db: &Connection, term: &str) -> i64 {
    db.query_row(
        "select count(*) from note_catalog_block_search where note_catalog_block_search match ?1",
        [format!("\"{term}\"")],
        |row| row.get(0),
    )
    .unwrap()
}
fn assert_integrity(db: &Connection) {
    for table in ["note_catalog_block_search", "note_catalog_metadata_search"] {
        db.execute(
            &format!("insert into {table}({table},rank) values('integrity-check',1)"),
            [],
        )
        .unwrap();
    }
    assert!(
        !db.prepare("pragma foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap()
    );
}
#[test]
fn catalog_text_uses_the_shared_frontend_semantic_fixtures() {
    let fixtures: Vec<Value> = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/contracts/note-catalog-text.json"
    )))
    .unwrap();
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    for fixture in fixtures {
        let id = super::tests::reading(&mut db, "page_note", fixture["quote"].as_str().unwrap());
        let document: NoteDocument = serde_json::from_value(fixture["document"].clone()).unwrap();
        save(&mut db, &id, document.clone());
        let texts: HashMap<String, String> = db
            .prepare("select block_id,search_text from note_catalog_blocks where page_id=?1")
            .unwrap()
            .query_map([&id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        fn visit(
            blocks: &[NoteDocumentBlock],
            texts: &HashMap<String, String>,
            output: &mut Vec<String>,
        ) {
            for block in blocks {
                let text = &texts[&block.id];
                if !text.is_empty() {
                    output.push(text.clone());
                }
                visit(&block.children, texts, output);
            }
        }
        let mut lines = Vec::new();
        visit(&document.blocks, &texts, &mut lines);
        assert_eq!(
            lines.join("\n"),
            fixture["expectedText"].as_str().unwrap(),
            "{}",
            fixture["name"]
        );
        let title: String = db
            .query_row(
                "select title from note_page_catalog where id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            title,
            fixture["expectedTitle"].as_str().unwrap(),
            "{}",
            fixture["name"]
        );
    }
    assert_integrity(&db);
}
#[test]
fn catalog_text_and_fts_follow_saved_changes_rollbacks_and_deletion() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    let mut document = load_document_from(&db, &id).unwrap().document;
    document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":"needle123","styles":[]}]});
    save(&mut db, &id, document.clone());
    assert_eq!(body_count(&db, "needle123"), 1);
    let tx = db.transaction().unwrap();
    tx.execute(
        "update note_blocks set content_json=?2 where page_id=?1 and position=0",
        rusqlite::params![
            &id,
            json!({"kind":"inline","items":[{"type":"text","text":"rollback456","styles":[]}]})
                .to_string()
        ],
    )
    .unwrap();
    tx.execute(
        "update note_pages set revision=revision+1 where id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(body_count(&tx, "rollback456"), 1);
    tx.rollback().unwrap();
    assert_eq!(body_count(&db, "rollback456"), 0);
    assert_eq!(body_count(&db, "needle123"), 1);
    document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":"changed789","styles":[]}]});
    save(&mut db, &id, document);
    assert_eq!(body_count(&db, "needle123"), 0);
    assert_eq!(body_count(&db, "changed789"), 1);
    db.execute(
        "update note_pages set is_favorite=1,revision=revision+1 where id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(body_count(&db, "changed789"), 1);
    assert_integrity(&db);
    db.execute("delete from note_pages where id=?1", [&id])
        .unwrap();
    assert_eq!(body_count(&db, "changed789"), 0);
    assert_integrity(&db);
}
#[test]
fn catalog_upgrade_195_reconciles_rich_titles_and_preserves_authoritative_json() {
    let path = std::env::temp_dir().join(format!(
        "note-catalog-rich-upgrade-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 195);
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    let mut document = load_document_from(&db, &id).unwrap().document;
    document.blocks[0].content = json!({"kind":"inline","items":[{"type":"text","text":"核心","styles":["bold"]},{"type":"text","text":"概念","styles":[]}]});
    // Seed the deployed schema directly: current title writes require 204.
    let tx = db.transaction().unwrap();
    tx.execute("update note_blocks set content_json=?2,plain_text='核心 概念',revision=revision+1 where id=?1", rusqlite::params![document.blocks[0].id,serde_json::to_string(&document.blocks[0].content).unwrap()]).unwrap();
    tx.execute(
        "update note_pages set revision=revision+1 where id=?1",
        [&id],
    )
    .unwrap();
    tx.commit().unwrap();
    let before = serde_json::to_value(load_document_from(&db, &id).unwrap()).unwrap();
    assert_eq!(
        db.query_row(
            "select title from note_page_catalog where id=?1",
            [&id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "核心 概念"
    );
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        serde_json::to_value(load_document_from(&db, &id).unwrap()).unwrap(),
        before
    );
    assert_eq!(
        db.query_row(
            "select title from note_page_catalog where id=?1",
            [&id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "核心概念"
    );
    assert_integrity(&db);
    drop(db);
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        serde_json::to_value(load_document_from(&db, &id).unwrap()).unwrap(),
        before
    );
    assert_integrity(&db);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
