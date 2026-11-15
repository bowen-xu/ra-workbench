use super::{fixture::*, *};
use crate::runtime_plugins::RuntimePluginCatalog;
use std::fs;

#[test]
fn raw_read_preserves_original_four_values_without_begin_or_semantic_validation() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    assert!(db.is_autocommit());
    let initial = fixture.resources.read_raw(&db).unwrap();
    assert_eq!(initial.revision, 0);
    assert_eq!(initial.definitions_json, "[]");
    assert_eq!(initial.cache_json, None);
    assert!(!initial.legacy_imported);
    assert!(db.is_autocommit());
    let tx = db.transaction().unwrap();
    // Structurally decodable, semantically invalid definitions must still read.
    tx.execute(
        "update nal_query_collections set revision=7,definitions=?1,cache='null',legacy_imported=1",
        [r#"[{"id":"","name":"","conditions":[]}]"#],
    )
    .unwrap();
    let row = fixture.resources.read_raw(&tx).unwrap();
    assert_eq!(row.revision, 7);
    assert_eq!(row.cache_json.as_deref(), Some("null"));
    assert!(row.clone().decode().is_ok());
    assert!(!tx.is_autocommit());
    tx.rollback().unwrap();
    drop(db);
    assert_eq!(fixture.raw(), initial);
}

#[test]
fn missing_singleton_and_invalid_primitive_rows_never_become_empty_defaults() {
    let fixture = Fixture::new();
    let db = fixture.db.lock().unwrap();
    db.execute("delete from nal_query_collections", []).unwrap();
    let error = fixture.resources.read_raw(&db).unwrap_err();
    let QueryCause::Data(error) = error else {
        panic!("expected no-row data error")
    };
    assert_eq!(error.safe_cause, "query_returned_no_rows");
    db.execute_batch("pragma ignore_check_constraints=on")
        .unwrap();
    db.execute(
        "insert into nal_query_collections(id,definitions) values(1,x'5b5d')",
        [],
    )
    .unwrap();
    let error = fixture.resources.read_raw(&db).unwrap_err();
    assert!(matches!(error, QueryCause::Data(_)));
}

#[test]
fn query_read_keeps_original_first_row_i64_and_nonzero_bool_rules() {
    let fixture = Fixture::new();
    let db = fixture.db.lock().unwrap();
    db.execute_batch("alter table nal_query_collections rename to query_fixture_original;
        create view nal_query_collections as select -9223372036854775808 as revision,'[]' as definitions,null as cache,-2 as legacy_imported,1 as id
        union all select 1,'[]',null,abs(-9223372036854775808),1;").unwrap();
    let row = fixture.resources.read_raw(&db).unwrap();
    assert_eq!(row.revision, i64::MIN);
    assert!(row.legacy_imported);
    assert!(db.is_autocommit());
}

#[test]
fn cas_save_decodes_before_commit_and_reopens_same_sql_log_with_original_shape() {
    let fixture = Fixture::new();
    let stored = save(
        &mut fixture.db.lock().unwrap(),
        &fixture.resources,
        &plan(),
        0,
    )
    .unwrap();
    assert_eq!(stored.revision, 1);
    assert!(stored.content.legacy_imported);
    assert_eq!(
        serde_json::to_value(&stored).unwrap(),
        serde_json::json!({"revision":1,"definitions":[],"cache":{"synthetic":1},"legacyImported":true})
    );
    assert!(fixture.db.lock().unwrap().is_autocommit());
    assert_eq!(fixture.reopen().raw().decode().unwrap(), stored);
}

#[test]
fn returned_revision_is_the_actual_read_value_not_expected_plus_one() {
    let fixture = Fixture::new();
    fixture.db.lock().unwrap().execute_batch("create trigger query_revision after update on nal_query_collections begin update nal_query_collections set revision=revision+4 where id=1; end;").unwrap();
    let stored = save(
        &mut fixture.db.lock().unwrap(),
        &fixture.resources,
        &plan(),
        0,
    )
    .unwrap();
    assert_eq!(stored.revision, 5);
    assert_eq!(fixture.raw().revision, 5);
}

#[test]
fn cas_conflict_precedes_decode_and_confirmed_rollback_keeps_original_retry_text() {
    let fixture = Fixture::new();
    fixture
        .db
        .lock()
        .unwrap()
        .execute("update nal_query_collections set definitions='null'", [])
        .unwrap();
    let error = save(
        &mut fixture.db.lock().unwrap(),
        &fixture.resources,
        &plan(),
        -1,
    )
    .unwrap_err();
    assert!(matches!(error.cause, QueryCause::Conflict));
    assert_eq!(error.outcome, QueryOutcome::RolledBack);
    assert_eq!(error.to_string(), "查询集合已变化，请重试");
    assert_eq!(fixture.raw().definitions_json, "null");
}

#[test]
fn structural_decode_failure_after_update_rolls_back_the_revision_and_content() {
    let fixture = Fixture::new();
    let before = fixture.raw();
    fixture.db.lock().unwrap().execute_batch("create trigger query_bad_structure after update on nal_query_collections begin update nal_query_collections set definitions='null' where id=1; end;").unwrap();
    let error = save(
        &mut fixture.db.lock().unwrap(),
        &fixture.resources,
        &plan(),
        0,
    )
    .unwrap_err();
    assert!(matches!(error.cause, QueryCause::InvalidStructure));
    assert_eq!(error.outcome, QueryOutcome::RolledBack);
    assert_eq!(error.to_string(), "关系数据结构无效");
    assert_eq!(fixture.raw(), before);
}

#[test]
fn revision_overflow_uses_original_sqlite_then_i64_error_and_rolls_back() {
    let fixture = Fixture::new();
    fixture
        .db
        .lock()
        .unwrap()
        .execute("update nal_query_collections set revision=?1", [i64::MAX])
        .unwrap();
    let error = save(
        &mut fixture.db.lock().unwrap(),
        &fixture.resources,
        &plan(),
        i64::MAX,
    )
    .unwrap_err();
    assert!(matches!(error.cause, QueryCause::Data(_)));
    assert_eq!(error.outcome, QueryOutcome::RolledBack);
    assert_eq!(fixture.raw().revision, i64::MAX);
}

#[test]
fn both_fixed_resource_hashes_are_checked_without_compiled_sql_fallback() {
    let fixture = Fixture::new();
    let file = fixture
        .directory
        .path()
        .join("packages/literature/data/nal-query-save.v1.sql");
    let mut data = fs::read(&file).unwrap();
    data.push(b'\n');
    fs::write(file, data).unwrap();
    let catalog = RuntimePluginCatalog::new(Some(fixture.directory.path().join("packages")));
    let package = catalog.discover().unwrap().packages.remove(0);
    let error = match NalQueryResources::load(&catalog, &package) {
        Err(error) => error,
        Ok(_) => panic!("changed bytes accepted"),
    };
    assert_eq!(error.code, "runtime_plugin_data_digest_mismatch");
}
