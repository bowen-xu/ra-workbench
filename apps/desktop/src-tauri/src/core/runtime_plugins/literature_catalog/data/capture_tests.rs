use super::{fixture::*, *};

#[test]
fn get_captures_original_primitive_fields_related_order_and_no_transaction() {
    let fixture = Fixture::new();
    fixture.item("paper", "Original title", None, "2025-01-01");
    fixture.related("paper");
    let db = fixture.db.lock().unwrap();
    let observed = observe_reads(&db);
    let rows = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap();
    stop_observing(&db);
    assert!(db.is_autocommit());
    assert_eq!(rows.len(), 1);
    let main = &rows[0].main.rows[0];
    assert_eq!(main.len(), 21);
    assert_eq!(main[0], text("paper"));
    assert_eq!(main[2], text("Original title"));
    assert_eq!(main[4], DataValue::Null);
    assert_eq!(main[12], text("10.1000/TEST"));
    assert_eq!(main[16], DataValue::Null);
    assert_eq!(main[20], DataValue::Null);
    let related = rows[0].related.as_ref().unwrap();
    assert_eq!(texts(&related.creators), ["Alpha", "Zed"]);
    assert_eq!(
        related.creators.rows[0],
        [text("Alpha"), text("A"), text(""), text("author")]
    );
    assert_eq!(texts(&related.tags), ["Alpha", "zeta"]);
    assert_eq!(
        texts(&related.attachments),
        ["attachment-first", "attachment-later"]
    );
    assert_eq!(related.attachments.rows[0][3], DataValue::Integer(17));
    assert_eq!(
        texts(&related.project_ids),
        ["project-first", "project-later"]
    );
    assert_eq!(texts(&related.metadata_sources), ["abstractText", "title"]);
    let reads = observed.lock().unwrap();
    assert!(!reads.iter().any(|table| table == "TRANSACTION"));
    let first = |name| reads.iter().position(|table| table == name).unwrap();
    assert!(first("bibliographic_items") < first("creators"));
    assert!(first("creators") < first("tags"));
    assert!(first("tags") < first("attachments"));
    assert!(first("attachments") < first("project_items"));
    assert!(first("project_items") < first("metadata_provenance"));
}

#[test]
fn missing_main_stops_related_and_later_item_reads() {
    let fixture = Fixture::new();
    fixture.item("later", "Later title", Some(2024), "now");
    let db = fixture.db.lock().unwrap();
    // A later related query would fail. The first missing main remains an
    // observation for Native NotFound, rather than being hidden by that error.
    db.execute_batch("drop table creators").unwrap();
    let observed = observe_reads(&db);
    let rows = capture_items(&db, &fixture.resources, &["missing".into(), "later".into()]).unwrap();
    stop_observing(&db);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].main.rows.is_empty());
    assert!(rows[0].related.is_none());
    let reads = observed.lock().unwrap();
    assert!(!reads.iter().any(|table| {
        [
            "creators",
            "tags",
            "attachments",
            "project_items",
            "metadata_provenance",
        ]
        .contains(&table.as_str())
    }));
}

#[test]
fn invalid_main_value_precedes_a_related_prepare_failure_without_leaking_values() {
    let fixture = Fixture::new();
    fixture.item("paper", "private-original-title", None, "now");
    let db = fixture.db.lock().unwrap();
    db.execute_batch("update bibliographic_items set title=x'ff'; drop table creators;")
        .unwrap();
    let error = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap_err();
    assert_eq!(error.resource_id, "literature.catalog.item-main");
    assert_eq!(error.phase, "read");
    assert_eq!(error.code, "plugin_data_sql_failed");
    let diagnostic = serde_json::to_string(&error).unwrap();
    assert!(!diagnostic.contains("private-original-title"));
    assert!(!diagnostic.contains("select b.id"));
    assert!(db.is_autocommit());
}

#[test]
fn creator_value_failure_precedes_tags_prepare_failure() {
    let fixture = Fixture::new();
    fixture.item("paper", "Title", None, "now");
    fixture.related("paper");
    let db = fixture.db.lock().unwrap();
    db.execute_batch(
        "update creators set family_name=x'ff'; drop table item_tags; drop table tags;",
    )
    .unwrap();
    let error = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap_err();
    assert_eq!(error.resource_id, "literature.catalog.item-creators");
    assert_eq!(error.phase, "read");
}

#[test]
fn original_i32_year_range_and_u64_attachment_range_fail_at_the_original_read() {
    let fixture = Fixture::new();
    fixture.item("paper", "Title", Some(i64::from(i32::MAX) + 1), "now");
    fixture.related("paper");
    let db = fixture.db.lock().unwrap();
    let error = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap_err();
    assert_eq!(error.resource_id, "literature.catalog.item-main");
    assert_eq!(error.phase, "read");
    db.execute_batch(
        "update bibliographic_items set issued_year=2024;
        pragma ignore_check_constraints=on;update content_objects set byte_size=-1;
        pragma ignore_check_constraints=off;drop table project_items;",
    )
    .unwrap();
    let error = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap_err();
    assert_eq!(error.resource_id, "literature.catalog.item-attachments");
    assert_eq!(error.phase, "read");
}

#[test]
fn original_bool_conversion_normalizes_every_nonzero_integer() {
    let fixture = Fixture::new();
    fixture.item("paper", "Title", Some(2024), "now");
    fixture.related("paper");
    let db = fixture.db.lock().unwrap();
    db.execute_batch(
        "pragma ignore_check_constraints=on;
        update metadata_provenance set confirmed_by_user=-2 where field_name='title';
        update metadata_provenance set confirmed_by_user=0 where field_name='abstractText';
        pragma ignore_check_constraints=off;",
    )
    .unwrap();
    let rows = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap();
    let provenance = &rows[0].related.as_ref().unwrap().metadata_sources.rows;
    assert_eq!(provenance[0][2], DataValue::Integer(0));
    assert_eq!(provenance[1][2], DataValue::Integer(1));
}

#[test]
fn item_main_only_steps_the_first_row_like_original_query_row() {
    let fixture = Fixture::new();
    let db = fixture.db.lock().unwrap();
    // The original PK normally makes this impossible. A view fixture exposes
    // a malformed second result to prove this Core binding uses FirstOptional.
    db.execute_batch("alter table bibliographic_items rename to original_items;
        create view bibliographic_items as
        select 'paper' id,'journalArticle' item_type,'First' title,'' abstract_text,null issued_year,
        '' container_title,'' conference_name,'' proceedings_title,'' place,'' volume,'' issue,'' pages,
        '' url,'' language,'' bibtex_key,null raw_bibtex,'unread' reading_status,
        'now' created_at,'now' updated_at,null trashed_at,null archived_at
        union all select 'paper','journalArticle',x'ff','',null,'','','','','','','','','','',null,'unread','now','now',null,null;").unwrap();
    let rows = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap();
    assert_eq!(rows[0].main.rows.len(), 1);
    assert_eq!(rows[0].main.rows[0][2], text("First"));
    assert!(rows[0].related.is_some());
}

#[test]
fn reads_borrow_the_callers_uncommitted_values_and_leave_rollback_to_that_caller() {
    let fixture = Fixture::new();
    fixture.item("paper", "Before", None, "now");
    let db = fixture.db.lock().unwrap();
    db.execute_batch("begin immediate;update bibliographic_items set title='Uncommitted';")
        .unwrap();
    let rows = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap();
    assert_eq!(rows[0].main.rows[0][2], text("Uncommitted"));
    assert!(!db.is_autocommit());
    db.execute_batch("rollback").unwrap();
    assert!(db.is_autocommit());
    let rows = capture_items(&db, &fixture.resources, &["paper".into()]).unwrap();
    assert_eq!(rows[0].main.rows[0][2], text("Before"));
}

#[test]
fn every_loaded_sql_resource_is_bound_to_its_frozen_package_hash() {
    let directory = tempfile::tempdir().unwrap();
    drop(package(directory.path()));
    std::fs::write(
        directory
            .path()
            .join("packages/literature/data/catalog-item-tags.v1.sql"),
        "select 'changed'",
    )
    .unwrap();
    let catalog = RuntimePluginCatalog::new(Some(directory.path().join("packages")));
    let package = catalog.discover().unwrap().packages.remove(0);
    let error = match CatalogResources::load(&catalog, &package) {
        Ok(_) => panic!("changed package SQL must not be accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, "runtime_plugin_data_digest_mismatch");
    assert_eq!(
        error.resource.as_deref(),
        Some("data/catalog-item-tags.v1.sql")
    );
}
