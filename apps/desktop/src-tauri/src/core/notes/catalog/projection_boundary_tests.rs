use super::*;
use crate::notes::document_repository::load_document_from;
use serde_json::json;

#[test]
fn invalid_blocks_preserve_canonical_content_and_expose_safe_projection_diagnostics() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "fallback quote");
    let block = load_document_from(&db, &id).unwrap().document.blocks[0]
        .id
        .clone();
    db.execute(
        "delete from note_blocks where page_id=?1 and id<>?2",
        rusqlite::params![id, block],
    )
    .unwrap();
    let invalid = json!({"kind":"inline","items":[{"type":"text","text":"private-needle","styles":["unknown"]}]}).to_string();
    db.execute(
        "update note_blocks set content_json=?2 where id=?1",
        rusqlite::params![block, invalid],
    )
    .unwrap();
    db.execute(
        "update note_pages set revision=revision+1 where id=?1",
        [&id],
    )
    .unwrap();
    let (code, cause): (String, String) = db
        .query_row(
            "select error_code,cause from note_catalog_projection_errors where block_id=?1",
            [&block],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(code, "note_catalog_invalid_text");
    assert_eq!(cause, "文本样式无效");
    assert!(
        !db.prepare("select 1 from note_catalog_blocks where block_id=?1")
            .unwrap()
            .exists([&block])
            .unwrap()
    );
    assert_eq!(
        db.query_row(
            "select content_json from note_blocks where id=?1",
            [&block],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        invalid
    );
    assert_eq!(
        query_from(&db, super::tests::query()).unwrap().items[0].title,
        "fallback quote"
    );
    // A valid rewrite clears the error and re-enters the same persisted index.
    let valid = json!({"kind":"inline","items":[{"type":"text","text":"corrected","styles":[]}]})
        .to_string();
    db.execute(
        "update note_blocks set content_json=?2 where id=?1",
        rusqlite::params![block, valid],
    )
    .unwrap();
    assert!(
        !db.prepare("select 1 from note_catalog_projection_errors where block_id=?1")
            .unwrap()
            .exists([&block])
            .unwrap()
    );
    assert_eq!(
        db.query_row(
            "select search_text from note_catalog_blocks where block_id=?1",
            [&block],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "corrected"
    );
}

#[test]
fn future_blocks_only_index_the_same_placeholder_as_the_frontend() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let id = super::tests::reading(&mut db, "page_note", "");
    let block = load_document_from(&db, &id).unwrap().document.blocks[0]
        .id
        .clone();
    let raw = json!({"secret":"opaque-future-body"}).to_string();
    db.execute(
        "update note_blocks set block_type='future-v2',content_json=?2 where id=?1",
        rusqlite::params![block, raw],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "select search_text from note_catalog_blocks where block_id=?1",
            [&block],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "[不支持的块：future-v2]"
    );
    assert_eq!(
        db.query_row(
            "select content_json from note_blocks where id=?1",
            [&block],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
}

#[test]
fn projection_function_is_safe_in_untrusted_schema_mode_and_rejects_malformed_json() {
    let db = Connection::open_in_memory().unwrap();
    register_projection_functions(&db).unwrap();
    db.execute_batch("pragma trusted_schema=off; create table input(kind text,props text,content text); create view projection as select ra_note_block_projection_v1(kind,props,content) as value from input; insert into input values('paragraph','{}','invalid-json');").unwrap();
    let result: String = db
        .query_row("select value from projection", [], |row| row.get(0))
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["errorCode"], "note_catalog_invalid_text");
    assert_eq!(result["cause"], "笔记块内容 JSON 无效");
    assert_eq!(result["text"], "");
}
