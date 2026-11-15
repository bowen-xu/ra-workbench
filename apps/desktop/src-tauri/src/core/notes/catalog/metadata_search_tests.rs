use super::*;
use crate::notes::model::NoteSearchQuery;
use serde_json::json;

fn fixture(db: &mut Connection) -> String {
    crate::notes::tests::insert_paper_fixture(db);
    let page = super::tests::reading(db, "highlight", "source quote");
    db.execute_batch("insert into tags(id,name) values('tag','oldtagtoken');
      insert into research_projects(id,title,summary,created_at) values('project','oldprojecttoken','','now');
      insert into creators(id,family_name,given_name) values('creator','Oldfamilytoken','Given');
      insert into item_creators(item_id,creator_id,role,position) values('paper-1','creator','author',0);").unwrap();
    db.execute(
        "insert into note_page_tags(page_id,tag_id) values(?1,'tag')",
        [&page],
    )
    .unwrap();
    db.execute(
        "insert into project_note_pages(project_id,page_id,added_at) values('project',?1,'now')",
        [&page],
    )
    .unwrap();
    page
}
fn hits(db: &Connection, text: &str) -> usize {
    let query: NoteSearchQuery = serde_json::from_value(json!({"search":text,"limit":20})).unwrap();
    super::legacy_search::search_from(db, query)
        .unwrap()
        .items
        .len()
}
fn authority(db: &Connection) -> (i64, Vec<String>) {
    let revision = db
        .query_row("select sum(revision) from note_pages", [], |row| row.get(0))
        .unwrap();
    let mut statement = db
        .prepare("select content_json from note_blocks order by id")
        .unwrap();
    let content = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap();
    (revision, content)
}
fn integrity(db: &Connection) {
    db.execute("insert into note_catalog_metadata_search(note_catalog_metadata_search,rank) values('integrity-check',1)",[]).unwrap();
}

#[test]
fn metadata_fields_update_incrementally_and_roll_back_with_their_canonical_metadata() {
    let mut db = crate::notes::tests::migrated_connection();
    let page = fixture(&mut db);
    let before = authority(&db);
    for text in ["oldtagtoken", "oldprojecttoken", "Oldfamilytoken"] {
        assert_eq!(hits(&db, text), 1, "{text}");
    }
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("update tags set name='rollbacktoken' where id='tag'", [])
        .unwrap();
    tx.rollback().unwrap();
    assert_eq!(hits(&db, "rollbacktoken"), 0);
    assert_eq!(hits(&db, "oldtagtoken"), 1);
    db.execute_batch(
        "update tags set name='newtagtoken' where id='tag';
      update research_projects set title='newprojecttoken' where id='project';
      update creators set family_name='Newfamilytoken' where id='creator';",
    )
    .unwrap();
    for text in ["oldtagtoken", "oldprojecttoken", "Oldfamilytoken"] {
        assert_eq!(hits(&db, text), 0, "{text}");
    }
    for text in ["newtagtoken", "newprojecttoken", "Newfamilytoken"] {
        assert_eq!(hits(&db, text), 1, "{text}");
    }
    db.execute(
        "update creators set literal_name='Corporateauthortoken' where id='creator'",
        [],
    )
    .unwrap();
    assert_eq!(hits(&db, "Corporateauthortoken"), 1);
    assert_eq!(
        hits(&db, "Newfamilytoken"),
        0,
        "literal names retain the original author precedence"
    );
    db.execute("delete from note_page_tags where page_id=?1", [&page])
        .unwrap();
    db.execute("delete from project_note_pages where page_id=?1", [&page])
        .unwrap();
    db.execute("delete from item_creators where item_id='paper-1'", [])
        .unwrap();
    for text in ["newtagtoken", "newprojecttoken", "Newfamilytoken"] {
        assert_eq!(hits(&db, text), 0, "{text}");
    }
    assert_eq!(hits(&db, "Corporateauthortoken"), 0);
    assert_eq!(authority(&db), before);
    integrity(&db);
}

#[test]
fn source_state_is_searchable_and_follows_paper_lifecycle_without_rebuilding() {
    let mut db = crate::notes::tests::migrated_connection();
    let page = fixture(&mut db);
    let state = crate::notes::reading_note_repository::reading_note_metadata_from(&db, &page)
        .unwrap()
        .source
        .state;
    assert_eq!(hits(&db, &state), 1);
    db.execute(
        "update bibliographic_items set trashed_at='now' where id='paper-1'",
        [],
    )
    .unwrap();
    assert_eq!(hits(&db, "paper_in_trash"), 1);
    assert_eq!(hits(&db, &state), 0);
    db.execute(
        "update bibliographic_items set trashed_at=null where id='paper-1'",
        [],
    )
    .unwrap();
    assert_eq!(hits(&db, "paper_in_trash"), 0);
    assert_eq!(hits(&db, &state), 1);
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'bb/changed.pdf','application/pdf',10,'now')",["b".repeat(64)]).unwrap();
    db.execute(
        "update attachments set content_sha256=?1 where id='attachment-1'",
        ["b".repeat(64)],
    )
    .unwrap();
    assert_eq!(hits(&db, "needs_relocation"), 1);
    assert_eq!(hits(&db, &state), 0);
    db.execute("delete from bibliographic_items where id='paper-1'", [])
        .unwrap();
    assert_eq!(hits(&db, "unavailable"), 1);
    assert_eq!(hits(&db, "needs_relocation"), 0);
    assert_eq!(hits(&db, "Oldfamilytoken"), 0);
    integrity(&db);
}

#[test]
fn metadata_203_upgrades_the_frozen_202_schema_and_reopens_without_rewriting_documents() {
    let path = std::env::temp_dir().join(format!(
        "note-metadata-upgrade-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 202);
    fixture(&mut db);
    let before = authority(&db);
    crate::notes::search::rebuild_note_index_from(&mut db).unwrap();
    let prior: i64 = db
        .query_row(
            "select count(*) from note_search where note_search match 'oldtagtoken'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(prior, 1, "the old search contract includes tag names");
    crate::storage::migrate(&mut db).unwrap();
    for text in ["oldtagtoken", "oldprojecttoken", "Oldfamilytoken"] {
        assert_eq!(hits(&db, text), 1, "{text}");
    }
    assert_eq!(authority(&db), before);
    integrity(&db);
    drop(db);
    let mut reopened = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut reopened).unwrap();
    assert_eq!(authority(&reopened), before);
    assert_eq!(hits(&reopened, "oldtagtoken"), 1);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn metadata_203_failure_preserves_the_old_projection_schema_and_allows_retry() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 202);
    fixture(&mut db);
    let before = authority(&db);
    let old_view: String = db
        .query_row(
            "select sql from sqlite_schema where name='note_catalog_metadata_text_values'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Deliberate schema collision after the first new indexes have been created.
    db.execute_batch("create index note_catalog_attachment_reverse on reading_note_sources(attachment_id_snapshot);").unwrap();
    assert!(crate::storage::migrate(&mut db).is_err());
    let version: i64 = db
        .query_row("select max(version) from schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 202);
    assert_eq!(authority(&db), before);
    assert_eq!(
        db.query_row(
            "select sql from sqlite_schema where name='note_catalog_metadata_text_values'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        old_view
    );
    db.execute_batch("drop index note_catalog_attachment_reverse")
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(hits(&db, "oldtagtoken"), 1);
    assert_eq!(authority(&db), before);
}

#[test]
fn old_metadata_preparation_is_deferred_batched_and_resumable_after_failure() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 202);
    fixture(&mut db);
    for i in 0..250 {
        db.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'metadata','2026-10-24T00:00:00Z','2026-10-24T00:00:00Z')",[format!("pending-{i:04}")]).unwrap();
    }
    let before = authority(&db);
    db.execute(
        "update note_pages set status='archived' where id='pending-0248'",
        [],
    )
    .unwrap();
    db.execute(
        "update note_pages set status='trashed' where id='pending-0249'",
        [],
    )
    .unwrap();
    let size_before: i64 = db
        .query_row(
            "select sum(length(search_text)) from note_catalog_metadata_text",
            [],
            |row| row.get(0),
        )
        .unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let pending = |db: &Connection| {
        db.query_row(
            "select count(*) from note_catalog_metadata_pending",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(pending(&db), 251);
    assert_eq!(
        db.query_row(
            "select sum(length(search_text)) from note_catalog_metadata_text",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        size_before
    );
    assert_eq!(
        super::query_from(&db, super::tests::query())
            .unwrap()
            .items
            .len(),
        20
    );
    assert_eq!(
        pending(&db),
        251,
        "browsing cannot start the search metadata preparation"
    );
    db.execute_batch("create trigger fail_metadata_batch before delete on note_catalog_metadata_pending when old.page_id='pending-0100' begin select raise(abort,'synthetic metadata batch failure'); end;").unwrap();
    let query: NoteSearchQuery = serde_json::from_value(json!({"search":"oldtagtoken"})).unwrap();
    assert!(super::legacy_search::search_from(&db, query).is_err());
    assert_eq!(
        pending(&db),
        151,
        "only the first 100-row transaction may commit"
    );
    assert_eq!(authority(&db), before);
    db.execute_batch("drop trigger fail_metadata_batch")
        .unwrap();
    assert_eq!(hits(&db, "oldtagtoken"), 1);
    assert_eq!(
        pending(&db),
        2,
        "active search must leave archived/trash preparation pending"
    );
    super::query_from(
        &db,
        PageCatalogQuery {
            search: Some("oldtagtoken".to_owned()),
            view: "archive".to_owned(),
            ..super::tests::query()
        },
    )
    .unwrap();
    assert_eq!(
        pending(&db),
        1,
        "global live search still excludes trash preparation"
    );
    super::query_from(
        &db,
        PageCatalogQuery {
            search: Some("oldtagtoken".to_owned()),
            view: "trash".to_owned(),
            ..super::tests::query()
        },
    )
    .unwrap();
    assert_eq!(pending(&db), 0);
    assert_eq!(authority(&db), before);
    integrity(&db);
}

#[test]
fn native_catalog_dispatch_prepares_one_batch_and_resumes_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("catalog.sqlite");
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 202);
    fixture(&mut db);
    for i in 0..250 {
        db.execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'metadata','2026-10-24T00:00:00Z','2026-10-24T00:00:00Z')", [format!("pending-{i:04}")]).unwrap();
    }
    let before = authority(&db);
    crate::storage::migrate(&mut db).unwrap();
    let pending = |db: &Connection| {
        db.query_row(
            "select count(*) from note_catalog_metadata_pending",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let query = || PageCatalogQuery {
        search: Some("oldtagtoken".into()),
        ..super::tests::query()
    };
    assert_eq!(pending(&db), 251);
    assert!(matches!(
        super::query_with_preparation_budget(&db, query(), Some(1)),
        Err(NotesError::CatalogPreparing)
    ));
    assert_eq!(pending(&db), 151);
    assert_eq!(authority(&db), before);
    drop(db);
    // Hiding dispatches no further batches. Reopening observes durable progress,
    // and a failed next batch cannot erase or falsely complete the first batch.
    let mut db = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(pending(&db), 151);
    db.execute_batch("create trigger fail_native_batch before delete on note_catalog_metadata_pending begin select raise(abort,'synthetic batch failure'); end;").unwrap();
    assert!(matches!(
        super::query_with_preparation_budget(&db, query(), Some(1)),
        Err(NotesError::Database(_))
    ));
    assert_eq!(pending(&db), 151);
    db.execute_batch("drop trigger fail_native_batch").unwrap();
    assert!(matches!(
        super::query_with_preparation_budget(&db, query(), Some(1)),
        Err(NotesError::CatalogPreparing)
    ));
    assert_eq!(pending(&db), 51);
    let result = super::query_with_preparation_budget(&db, query(), Some(1)).unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(pending(&db), 0);
    assert_eq!(authority(&db), before);
    let hot = super::query_with_preparation_budget(&db, query(), Some(1)).unwrap();
    assert_eq!(hot.items.len(), 1);
    assert_eq!(hot.revision, result.revision);
    integrity(&db);
}
