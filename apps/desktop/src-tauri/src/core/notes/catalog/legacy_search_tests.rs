use super::*;
use rusqlite::{StatementStatus, params};
use serde_json::json;

fn query(value: serde_json::Value) -> NoteSearchQuery {
    serde_json::from_value(value).unwrap()
}

fn seed(db: &Connection, count: usize) {
    let tx = db.unchecked_transaction().unwrap();
    for i in 0..count {
        let id = format!("page-{i:04}");
        tx.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'metadata','2026-10-24T00:00:00Z','2026-10-24T00:00:00Z')",[&id]).unwrap();
        let text = if i == 1737 {
            "unique-needle-1737"
        } else {
            "unrelated text"
        };
        tx.execute("insert into note_blocks(id,scope_kind,page_id,block_type,props_json,content_json,plain_text,created_at,updated_at)values(?1,'page',?2,'paragraph','{}',?3,'legacy','now','now')",
            params![format!("block-{i:04}"),id,json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]}).to_string()]).unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn existing_search_uses_live_shared_projection_without_rebuilding_or_reading_canonical_blocks() {
    let db = crate::notes::tests::migrated_connection();
    seed(&db, 2400);
    db.execute_batch(
        "drop table note_search; alter table note_blocks rename to canonical_blocks_unavailable;",
    )
    .unwrap();
    db.pragma_update(None, "query_only", true).unwrap();
    let found = search_from(
        &db,
        query(json!({"search":"unique-needle-1737","limit":20})),
    )
    .unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].object_id, "page-1737");
    assert_eq!(found.items[0].snippet, "unique-needle-1737");
    assert_eq!(found.index_state, "ready");
    assert!(found.next_cursor.is_none());
    assert_eq!(
        crate::notes::search::index_state_from(&db).unwrap(),
        "dirty"
    );
}

#[test]
fn invalid_projection_blocks_search_in_its_status_without_blocking_browse() {
    let db = crate::notes::tests::migrated_connection();
    seed(&db, 1);
    db.execute(
        "update note_blocks set content_json='{\"kind\":\"table\"}' where id='block-0000'",
        [],
    )
    .unwrap();
    let failure = search_from(&db, query(json!({"search":"anything"})))
        .unwrap_err()
        .to_string();
    assert!(failure.contains("note_catalog_invalid_text"));
    assert!(!failure.contains("page-0000"));
    assert_eq!(search_from(&db, query(json!({}))).unwrap().items.len(), 1);
    assert!(
        search_from(
            &db,
            query(json!({"search":"anything", "status":"archived"}))
        )
        .unwrap()
        .items
        .is_empty()
    );
}

#[test]
fn compatibility_search_fts_plan_is_selective_and_returns_only_the_requested_page() {
    let db = crate::notes::tests::migrated_connection();
    seed(&db, 2400);
    let input = query(json!({"search":"unique-needle-1737","limit":20}));
    let terms = search::terms(input.search.as_deref().unwrap()).unwrap();
    let mut values = vec![
        Value::Text("active".to_owned()),
        Value::Text(String::new()),
        Value::Text(String::new()),
        Value::Integer(21),
    ];
    values.extend(terms.iter().map(|term| Value::Text(format!("\"{term}\""))));
    values.extend(terms.iter().cloned().map(Value::Text));
    let sql = sql(&input, &terms, &mut values);
    assert!(!sql.contains("content_json"));
    assert!(!sql.contains("from note_blocks"));
    assert!(!sql.contains("note_search"));
    let mut statement = db.prepare(&sql).unwrap();
    let count = statement
        .query_map(rusqlite::params_from_iter(values), |_| Ok(()))
        .unwrap()
        .count();
    assert_eq!(count, 1);
    let scans = statement.get_status(StatementStatus::FullscanStep);
    let steps = statement.get_status(StatementStatus::VmStep);
    eprintln!("legacy_shared_search rows={count} fullscan_steps={scans} vm_steps={steps}");
    assert!(scans < 64, "{scans}");
    assert!(steps < 3000, "{steps}");
}

#[test]
fn status_and_metadata_filters_apply_before_the_result_limit() {
    let db = crate::notes::tests::migrated_connection();
    seed(&db, 40);
    db.execute(
        "update note_pages set status='archived' where id<'page-0030'",
        [],
    )
    .unwrap();
    db.execute("insert into tags(id,name) values('tag','tag')", [])
        .unwrap();
    db.execute(
        "insert into note_page_tags(page_id,tag_id) values('page-0037','tag')",
        [],
    )
    .unwrap();
    db.execute("insert into research_projects(id,title,summary,created_at) values('project','project','','now')",[]).unwrap();
    db.execute(
        "insert into project_note_pages(project_id,page_id,added_at) values('project','page-0037','now')",
        [],
    )
    .unwrap();
    let found = search_from(
        &db,
        query(
            json!({"tagId":"tag","projectId":"project","limit":1,"paperId":"ignored-for-ordinary"}),
        ),
    )
    .unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].object_id, "page-0037");
    assert!(found.next_cursor.is_none());
    assert!(
        search_from(&db, query(json!({"projectId":"missing","tagId":"tag"})))
            .unwrap()
            .items
            .is_empty()
    );
    let first = search_from(&db, query(json!({"limit":2}))).unwrap();
    assert_eq!(first.items[0].object_id, "page-0030");
    assert_eq!(first.items.len(), 2);
    let second = search_from(&db, query(json!({"limit":20,"cursor":first.next_cursor}))).unwrap();
    assert_eq!(second.items.len(), 8);
    assert_eq!(second.items[0].object_id, "page-0032");
    assert!(second.next_cursor.is_none());
    assert!(
        search_from(&db, query(json!({"objectKinds":[]})))
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn reading_filters_and_source_metadata_do_not_hydrate_document_bodies() {
    let mut db = crate::notes::tests::migrated_connection();
    crate::notes::tests::insert_paper_fixture(&db);
    let page = super::super::tests::reading(&mut db, "highlight", "shared-quote");
    db.execute("update note_pages set is_favorite=1 where id=?1", [&page])
        .unwrap();
    let expected = reading_note_metadata_from(&db, &page).unwrap().source;
    db.execute_batch("alter table note_blocks rename to canonical_blocks_unavailable;")
        .unwrap();
    let found=search_from(&db,query(json!({"search":"shared-quote","objectKinds":["reading_note"],"paperId":"paper-1","noteType":"highlight","sourceState":expected.state,"isFavorite":true,"limit":1}))).unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(
        serde_json::to_value(found.items[0].source.as_ref().unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert!(
        search_from(
            &db,
            query(json!({"objectKinds":["reading_note"],"paperId":"another"}))
        )
        .unwrap()
        .items
        .is_empty()
    );
    db.execute(
        "update bibliographic_items set trashed_at='now' where id='paper-1'",
        [],
    )
    .unwrap();
    let found = search_from(
        &db,
        query(json!({"objectKinds":["reading_note"],"sourceState":"paper_in_trash"})),
    )
    .unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(
        found.items[0].source.as_ref().unwrap().state,
        "paper_in_trash"
    );
}

#[test]
fn canonical_updates_and_rollbacks_maintain_the_same_search_projection_without_a_rebuild() {
    let db = crate::notes::tests::migrated_connection();
    seed(&db, 2);
    let content=json!({"kind":"inline","items":[{"type":"text","text":"new-shared-search-term","styles":[]}]}).to_string();
    let input = || query(json!({"search":"new-shared-search-term"}));
    let tx = db.unchecked_transaction().unwrap();
    tx.execute(
        "update note_blocks set content_json=?1 where id='block-0000'",
        [&content],
    )
    .unwrap();
    tx.rollback().unwrap();
    assert!(search_from(&db, input()).unwrap().items.is_empty());
    db.execute(
        "update note_blocks set content_json=?1 where id='block-0000'",
        [&content],
    )
    .unwrap();
    let before:(i64,String)=db.query_row("select p.revision,b.content_json from note_pages p join note_blocks b on b.page_id=p.id where b.id='block-0000'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    let found = search_from(&db, input()).unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].object_id, "page-0000");
    let after:(i64,String)=db.query_row("select p.revision,b.content_json from note_pages p join note_blocks b on b.page_id=p.id where b.id='block-0000'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(after, before, "a search cannot author or save the note");
}
