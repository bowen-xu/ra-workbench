use crate::hbs_storage::{self, Commit};
use rusqlite::Connection;
use serde_json::{Value, json};

fn commit(
    db: &mut Connection,
    page: &str,
    id: &str,
    point: &Value,
    saved: Value,
    baseline: Value,
) -> Result<Value, String> {
    crate::hbs_storage::test_sources::commit(
        db,
        "note",
        page,
        Commit {
            id: id.into(),
            expected_version: point["version"].as_i64().unwrap(),
            saved,
            baseline,
        },
    )
}
#[test]
fn title_body_join_rolls_back_retries_and_reopens_with_history_and_preservation() {
    let path = std::env::temp_dir().join(format!("title-body-{}.sqlite", uuid::Uuid::new_v4()));
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let page = "00000000-0000-4000-8000-000000000289";
    let document = json!({"schemaVersion":1,"pageId":page,"title":"Title","blocks":[{"id":"body","type":"paragraph","props":{},"content":{"kind":"inline","items":[{"type":"text","text":"Accepted","styles":[]}]},"children":[]}]});
    let before = super::seed(
        &mut db,
        page,
        serde_json::from_value(document.clone()).unwrap(),
    )
    .unwrap();
    let saved = json!({"schemaVersion":1,"pageId":page,"title":"TitleCandidate","blocks":[]});
    let baseline = json!({"schemaVersion":1,"pageId":page,"title":"TitleAccepted","blocks":[]});
    db.execute_batch("create trigger fail_title_baseline before update on hbs_note_baselines begin select raise(abort,'title rollback'); end;").unwrap();
    assert!(
        commit(
            &mut db,
            page,
            "title-join",
            &before,
            saved.clone(),
            baseline.clone()
        )
        .unwrap_err()
        .contains("title rollback")
    );
    assert_eq!(hbs_storage::read(&db, "note", page).unwrap(), before);
    db.execute_batch("drop trigger fail_title_baseline")
        .unwrap();
    let receipt = commit(
        &mut db,
        page,
        "title-join",
        &before,
        saved.clone(),
        baseline.clone(),
    )
    .unwrap();
    assert_eq!(
        commit(
            &mut db,
            page,
            "title-join",
            &before,
            saved.clone(),
            baseline.clone()
        )
        .unwrap(),
        receipt
    );
    let checkpoint = json!({"format":1,"kind":"note","observed":receipt["snapshot"],"content":saved,"ancestor":saved,"baselineAncestor":baseline,"ordinary":[]});
    crate::hbs_storage::test_sources::preservation_save(&mut db, "note", page, checkpoint.clone())
        .unwrap();
    drop(db);
    let mut reopened = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut reopened).unwrap();
    assert_eq!(
        hbs_storage::read(&reopened, "note", page).unwrap(),
        receipt["snapshot"]
    );
    assert_eq!(
        crate::hbs_storage::test_sources::preservation_read(&reopened, "note", page).unwrap(),
        checkpoint
    );
    let title: String = reopened
        .query_row("select title from note_pages where id=?1", [page], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(title, "TitleCandidate");
    let prior: String = reopened
        .query_row(
            "select document_json from note_revisions where page_id=?1 and page_revision=?2",
            rusqlite::params![page, before["version"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&prior).unwrap(), document);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn old_body_only_documents_and_baselines_preserve_the_existing_title() {
    let mut db = crate::notes::tests::migrated_connection();
    let page = "00000000-0000-4000-8000-000000000290";
    let document = json!({"schemaVersion":1,"pageId":page,"blocks":[]});
    let before = super::seed(
        &mut db,
        page,
        serde_json::from_value(document.clone()).unwrap(),
    )
    .unwrap();
    db.execute(
        "update hbs_note_baselines set baseline_json=?2 where page_id=?1",
        rusqlite::params![page, document.to_string()],
    )
    .unwrap();
    let old = hbs_storage::read(&db, "note", page).unwrap();
    assert_eq!(old["saved"]["title"], before["saved"]["title"]);
    assert_eq!(old["baseline"]["title"], before["saved"]["title"]);
    let result = commit(
        &mut db,
        page,
        "legacy-body",
        &old,
        document.clone(),
        document,
    )
    .unwrap();
    assert_eq!(
        result["snapshot"]["saved"]["title"],
        before["saved"]["title"]
    );
}

#[test]
fn title_204_upgrades_frozen_203_without_rewriting_and_reopens_an_empty_title() {
    let path = std::env::temp_dir().join(format!("title-upgrade-{}.sqlite", uuid::Uuid::new_v4()));
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 203);
    let tx = db.transaction().unwrap();
    let page = crate::notes::page_creation::create_page_with_paper_in(
        &tx,
        crate::notes::model::NotePageDraft {
            title: "Old title".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        None,
    )
    .unwrap();
    let old_document = json!({"schemaVersion":1,"pageId":page,"blocks":[]});
    tx.execute(
        "insert into hbs_note_baselines(page_id,baseline_json) values(?1,?2)",
        rusqlite::params![page, old_document.to_string()],
    )
    .unwrap();
    tx.commit().unwrap();
    let authority = |db: &Connection| -> (String, String, String) {
        (db.query_row("select json_array(id,title,revision,created_at,updated_at) from note_pages where id=?1",[&page],|r|r.get(0)).unwrap(),
        db.query_row("select json_group_array(json_array(id,content_json,props_json)) from note_blocks where page_id=?1",[&page],|r|r.get(0)).unwrap(),
        db.query_row("select baseline_json from hbs_note_baselines where page_id=?1",[&page],|r|r.get(0)).unwrap())
    };
    let before = authority(&db);
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(authority(&db), before);
    let point = hbs_storage::read(&db, "note", &page).unwrap();
    assert_eq!(point["saved"]["title"], "Old title");
    let empty = json!({"schemaVersion":1,"pageId":page,"title":"","blocks":[]});
    commit(
        &mut db,
        &page,
        "empty-title",
        &point,
        empty.clone(),
        empty.clone(),
    )
    .unwrap();
    drop(db);
    let mut reopened = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut reopened).unwrap();
    let point = hbs_storage::read(&reopened, "note", &page).unwrap();
    assert_eq!(point["saved"], empty);
    assert_eq!(point["baseline"], empty);
    assert_eq!(
        crate::notes::document_repository::load_document_from(&reopened, &page)
            .unwrap()
            .page
            .title,
        "无标题"
    );
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn title_projection_reads_deployed_metadata_and_does_not_mask_invalid_schema() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 203);
    db.execute("insert into note_pages(id,title,position,created_at,updated_at) values('legacy-title','Original title',0,'now','now')", []).unwrap();
    let before = db.total_changes();
    let loaded = crate::notes::load_document_value(&db, "legacy-title").unwrap();
    assert_eq!(loaded.title.as_deref(), Some("Original title"));
    assert_eq!(db.total_changes(), before);
    assert!(!db.query_row("select exists(select 1 from pragma_table_info('note_pages') where name='document_title')", [], |row| row.get::<_,bool>(0)).unwrap());
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        crate::notes::load_document_value(&db, "legacy-title")
            .unwrap()
            .title,
        loaded.title
    );
    db.execute(
        "update note_pages set document_title='' where id='legacy-title'",
        [],
    )
    .unwrap();
    assert_eq!(
        crate::notes::load_document_value(&db, "legacy-title")
            .unwrap()
            .title
            .as_deref(),
        Some("")
    );
    db.execute_batch("pragma foreign_keys=off; drop table note_blocks;")
        .unwrap();
    assert!(crate::notes::load_document_value(&db, "legacy-title").is_err());
}
