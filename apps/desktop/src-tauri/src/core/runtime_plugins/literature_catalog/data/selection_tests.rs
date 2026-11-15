use super::{fixture::*, *};

#[test]
fn all_selection_preserves_original_active_filter_and_created_order_without_a_transaction() {
    let fixture = Fixture::new();
    fixture.item("older", "Older", None, "2024-01-01");
    fixture.item("newer", "Newer", None, "2025-01-01");
    fixture.item("archived", "Archived", None, "2026-01-01");
    fixture.item("trashed", "Trashed", None, "2027-01-01");
    let db = fixture.db.lock().unwrap();
    db.execute_batch(
        "update bibliographic_items set archived_at='now' where id='archived';
        update bibliographic_items set trashed_at='now' where id='trashed';",
    )
    .unwrap();
    let observed = observe_reads(&db);
    let prepared = capture_selection(&db, &fixture.resources, &CatalogQueryPlan::All).unwrap();
    recheck_selection(&db, &prepared).unwrap();
    stop_observing(&db);
    assert!(db.is_autocommit());
    assert_eq!(texts(&prepared.rows().primary), ["newer", "older"]);
    assert!(prepared.rows().year.is_none());
    assert!(
        !observed
            .lock()
            .unwrap()
            .iter()
            .any(|table| table == "TRANSACTION")
    );
}

#[test]
fn search_keeps_raw_fts_duplicates_and_year_stream_separate_for_native_selection() {
    let fixture = Fixture::new();
    fixture.item("fts", "Search hit", Some(2024), "2024-01-01");
    fixture.item("year-only", "Year-only", Some(2024), "2025-01-01");
    let db = fixture.db.lock().unwrap();
    db.execute_batch(
        "delete from literature_search;
        insert into literature_search(item_id,title) values('fts','2024'),('fts','2024');",
    )
    .unwrap();
    let prepared = capture_selection(
        &db,
        &fixture.resources,
        &CatalogQueryPlan::Search {
            fts_query: "\"2024\"".into(),
            year: Some(2024),
        },
    )
    .unwrap();
    assert_eq!(texts(&prepared.rows().primary), ["fts", "fts"]);
    let year = texts(prepared.rows().year.as_ref().unwrap());
    assert_eq!(year.len(), 2);
    assert!(year.contains(&"fts".into()));
    assert!(year.contains(&"year-only".into()));
    recheck_selection(&db, &prepared).unwrap();
}

#[test]
fn fts_without_year_does_not_read_the_year_query() {
    let fixture = Fixture::new();
    fixture.item("paper", "Title", Some(2024), "now");
    let db = fixture.db.lock().unwrap();
    db.execute_batch("delete from literature_search;insert into literature_search(item_id,title) values('paper','needle');").unwrap();
    let prepared = capture_selection(
        &db,
        &fixture.resources,
        &CatalogQueryPlan::Search {
            fts_query: "\"needle\"".into(),
            year: None,
        },
    )
    .unwrap();
    assert_eq!(texts(&prepared.rows().primary), ["paper"]);
    assert!(prepared.rows().year.is_none());
}

#[test]
fn recheck_detects_previously_empty_selector_becoming_nonempty() {
    let fixture = Fixture::new();
    let prepared = {
        let db = fixture.db.lock().unwrap();
        capture_selection(&db, &fixture.resources, &CatalogQueryPlan::All).unwrap()
    };
    assert!(prepared.rows().primary.rows.is_empty());
    fixture.item("paper", "New", None, "now");
    let error = recheck_selection(&fixture.db.lock().unwrap(), &prepared).unwrap_err();
    assert_eq!(error.code, "plugin_data_snapshot_changed");
    assert_eq!(error.phase, "recheck");
}

#[test]
fn recheck_compares_full_selector_order_and_year_absence_not_a_count_or_watermark() {
    let fixture = Fixture::new();
    fixture.item("a", "A", Some(2023), "2024-01-01");
    fixture.item("b", "B", Some(2023), "2025-01-01");
    let db = fixture.db.lock().unwrap();
    let all = capture_selection(&db, &fixture.resources, &CatalogQueryPlan::All).unwrap();
    db.execute(
        "update bibliographic_items set created_at='2026-01-01' where id='a'",
        [],
    )
    .unwrap();
    assert_eq!(
        recheck_selection(&db, &all).unwrap_err().code,
        "plugin_data_snapshot_changed"
    );
    let search = capture_selection(
        &db,
        &fixture.resources,
        &CatalogQueryPlan::Search {
            fts_query: "\"absent\"".into(),
            year: Some(2024),
        },
    )
    .unwrap();
    assert!(search.rows().primary.rows.is_empty());
    assert!(search.rows().year.as_ref().unwrap().rows.is_empty());
    db.execute(
        "update bibliographic_items set issued_year=2024 where id='b'",
        [],
    )
    .unwrap();
    let error = recheck_selection(&db, &search).unwrap_err();
    assert_eq!(error.code, "plugin_data_snapshot_changed");
    assert_eq!(error.resource_id, "literature.catalog.search-year-ids");
}

#[test]
fn selector_capture_owns_its_rows_and_releases_the_original_database_guard() {
    let fixture = Fixture::new();
    let captured = {
        let db = fixture.db.lock().unwrap();
        capture_selection(&db, &fixture.resources, &CatalogQueryPlan::All).unwrap()
    };
    let database = std::sync::Arc::clone(&fixture.db);
    std::thread::spawn(move || {
        let db = database.lock().unwrap();
        db.execute(
            "insert into note_pages(id,title,position,status,created_at,updated_at)
            values('unrelated-note','Core resource',0,'active','now','now')",
            [],
        )
        .unwrap();
        insert_item(&db, "paper", "After capture", None, "now");
    })
    .join()
    .unwrap();
    assert!(captured.rows().primary.rows.is_empty());
    let db = fixture.db.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from note_pages where id='unrelated-note'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        recheck_selection(&db, &captured).unwrap_err().code,
        "plugin_data_snapshot_changed"
    );
}

#[test]
fn fts_failure_happens_before_year_selection_and_returns_original_sqlite_category() {
    let fixture = Fixture::new();
    let db = fixture.db.lock().unwrap();
    let error = match capture_selection(
        &db,
        &fixture.resources,
        &CatalogQueryPlan::Search {
            fts_query: "\"private-unclosed".into(),
            year: Some(2024),
        },
    ) {
        Ok(_) => panic!("invalid FTS must fail"),
        Err(error) => error,
    };
    assert_eq!(error.resource_id, "literature.catalog.search-fts-ids");
    assert_eq!(error.code, "plugin_data_sql_failed");
    assert!(error.sqlite_extended_code.is_some());
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private-unclosed")
    );
    assert!(db.is_autocommit());
}
