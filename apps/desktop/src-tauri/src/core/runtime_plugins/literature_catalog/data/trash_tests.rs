use super::{fixture::*, *};

#[test]
fn trash_selectors_keep_original_order_visibility_and_the_same_related_objects() {
    let fixture = Fixture::new();
    let resources = fixture.trash_resources();
    fixture.item("active", "Active", None, "2027");
    fixture.item("older", "Older", None, "2028");
    fixture.item("recent", "Recent", None, "2026");
    fixture.item("archived", "Archived", None, "2029");
    fixture.related("recent");
    let db = fixture.db.lock().unwrap();
    db.execute_batch("update bibliographic_items set trashed_at='2025-01-01' where id='older';
        update bibliographic_items set trashed_at='2025-01-02',archived_at='archive' where id='recent';
        update bibliographic_items set archived_at='archive' where id='archived';").unwrap();
    let observed = observe_reads(&db);
    let selected = capture_selection(&db, &resources, &CatalogQueryPlan::All).unwrap();
    assert_eq!(texts(&selected.rows().primary), ["recent", "older"]);
    recheck_selection(&db, &selected).unwrap();
    let ids = texts(&selected.rows().primary);
    assert!(
        capture_items(&db, &resources, &ids).unwrap()[0]
            .main
            .rows
            .is_empty()
    );
    let items = capture_trashed_items(&db, &resources, &ids).unwrap();
    stop_observing(&db);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].main.rows[0][0], text("recent"));
    assert_eq!(items[0].main.rows[0][20], text("2025-01-02"));
    let related = items[0].related.as_ref().unwrap();
    assert_eq!(related.attachments.rows.len(), 2);
    assert_eq!(related.creators.rows.len(), 2);
    assert_eq!(
        texts(&related.project_ids),
        ["project-first", "project-later"]
    );
    assert!(db.is_autocommit());
    assert!(
        !observed
            .lock()
            .unwrap()
            .iter()
            .any(|table| table == "TRANSACTION")
    );
}

#[test]
fn trash_search_retains_fts_duplicates_and_year_append_inputs_excluding_active_rows() {
    let fixture = Fixture::new();
    let resources = fixture.trash_resources();
    for id in ["active", "fts", "year-only"] {
        fixture.item(id, id, Some(2024), "now");
    }
    let db = fixture.db.lock().unwrap();
    db.execute_batch("update bibliographic_items set trashed_at='trash' where id<>'active';
        delete from literature_search;
        insert into literature_search(item_id,title) values('active','2024'),('fts','2024'),('fts','2024');").unwrap();
    let selected = capture_selection(
        &db,
        &resources,
        &CatalogQueryPlan::Search {
            fts_query: "\"2024\"".into(),
            year: Some(2024),
        },
    )
    .unwrap();
    assert_eq!(texts(&selected.rows().primary), ["fts", "fts"]);
    let year = texts(selected.rows().year.as_ref().unwrap());
    assert_eq!(year.len(), 2);
    assert!(year.contains(&"fts".into()) && year.contains(&"year-only".into()));
    recheck_selection(&db, &selected).unwrap();
    db.execute(
        "update bibliographic_items set trashed_at=null where id='year-only'",
        [],
    )
    .unwrap();
    let error = recheck_selection(&db, &selected).unwrap_err();
    assert_eq!(error.code, "plugin_data_snapshot_changed");
    assert_eq!(
        error.resource_id,
        "literature.catalog.search-trash-year-ids"
    );
}
