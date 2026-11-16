use super::*;
use rusqlite::functions::FunctionFlags;
use serde_json::json;
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

fn package_fixture() -> (
    tempfile::TempDir,
    RuntimePluginCatalog,
    RuntimePluginPackage,
) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("literature");
    fs::create_dir_all(directory.join("data")).unwrap();
    fs::write(
        directory.join("manifest.json"),
        r#"{"id":"literature_library","entry":"entry.js","styles":[]}"#,
    )
    .unwrap();
    fs::write(directory.join("entry.js"), "export {};").unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in ["header", "creators", "identifiers"] {
        let file = format!("context-metadata-{name}.v1.sql");
        fs::copy(source.join(&file), directory.join("data").join(file)).unwrap();
    }
    let catalog = RuntimePluginCatalog::new(Some(root.path().into()));
    let package = catalog.discover().unwrap().packages.remove(0);
    (root, catalog, package)
}

fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "create table bibliographic_items(
            id text primary key,title text,item_type text,abstract_text text,
            issued_year integer,container_title text,volume text,issue text,
            pages text,url text,language text,bibtex_key text,
            archived_at text,trashed_at text);
        create table creators(id text primary key,family_name text,given_name text,literal_name text);
        create table item_creators(item_id text,creator_id text,role text,position integer);
        create table identifiers(id text primary key,item_id text,kind text,display_value text);
        insert into bibliographic_items(id,title,item_type,issued_year,volume,language,bibtex_key)
            values('paper','题录 Ω','article',2026,'','zh','key');
        insert into creators values('second','B','Given',''),('first','','','机构');
        insert into item_creators values('paper','second','editor',2),('paper','first','author',1);
        insert into identifiers values('2','paper','doi','second'),('1','paper','doi','first'),('0','paper','url','url');",
    ).unwrap();
    db
}

fn data_cause(failure: Failure<PaperMetadataError>) -> Box<PluginDataError> {
    match failure.cause {
        FailureCause::SourceData(cause) => cause,
        other => panic!("expected actual fixed-data failure, got {other:?}"),
    }
}

#[test]
fn captures_original_values_order_archived_visibility_without_writes() {
    let (_root, catalog, package) = package_fixture();
    let resources = ContextMetadataResources::load(&catalog, &package).unwrap();
    let db = database();
    let changes = db.total_changes();
    let captured = resources.capture(&db, "paper").unwrap();
    assert_eq!(resources.package_key(), package.package_key);
    assert_eq!(captured.input.paper_id, "paper");
    assert_eq!(
        captured.input.base,
        json!({
            "title":"题录 Ω", "itemType":"article", "abstract":null, "year":2026,
            "container":null, "volume":"", "issue":null, "pages":null, "url":null,
            "language":"zh", "bibtexKey":"key"
        })
    );
    assert_eq!(
        captured.input.creators,
        [["  机构", "author"], ["B Given ", "editor"]]
    );
    assert_eq!(
        captured.input.identifiers,
        [["doi", "first"], ["doi", "second"], ["url", "url"]]
    );
    assert_eq!(db.total_changes(), changes);
    assert!(db.is_autocommit());
    db.execute("update bibliographic_items set archived_at='now'", [])
        .unwrap();
    assert_eq!(resources.capture(&db, "paper").unwrap(), captured);
    db.execute("update bibliographic_items set issued_year=null", [])
        .unwrap();
    assert!(resources.capture(&db, "paper").unwrap().input.base["year"].is_null());
    db.execute("update bibliographic_items set trashed_at='now'", [])
        .unwrap();
    assert_eq!(
        resources.capture(&db, "paper").unwrap_err().error,
        PaperMetadataError::SourceUnavailable
    );
}

#[test]
fn header_missing_sql_and_string_errors_keep_original_stage_and_safe_cause() {
    let (_root, catalog, package) = package_fixture();
    let resources = ContextMetadataResources::load(&catalog, &package).unwrap();
    for db in [Connection::open_in_memory().unwrap(), database()] {
        let failure = resources.capture(&db, "missing").unwrap_err();
        assert_eq!(failure.error, PaperMetadataError::SourceUnavailable);
        assert_eq!(
            data_cause(failure).resource_id,
            "literature.context-metadata.header"
        );
    }
    let db = database();
    db.create_scalar_function("json_object", -1, FunctionFlags::default(), |_| Ok(7_i64))
        .unwrap();
    db.execute_batch("drop table item_creators; drop table identifiers")
        .unwrap();
    let failure = resources.capture(&db, "paper").unwrap_err();
    assert_eq!(failure.error, PaperMetadataError::SourceUnavailable);
    let cause = data_cause(failure);
    assert_eq!(cause.resource_id, "literature.context-metadata.header");
    assert_eq!(cause.package_key, package.package_key);
    assert!(!cause.safe_cause.is_empty());
}

#[test]
fn json_decode_precedes_both_related_reads_and_has_original_error_class() {
    let (_root, catalog, package) = package_fixture();
    let resources = ContextMetadataResources::load(&catalog, &package).unwrap();
    let db = database();
    db.create_scalar_function("json_object", -1, FunctionFlags::default(), |_| {
        Ok("not json".to_owned())
    })
    .unwrap();
    db.execute_batch("drop table item_creators; drop table identifiers")
        .unwrap();
    let failure = resources.capture(&db, "paper").unwrap_err();
    assert_eq!(failure.error, PaperMetadataError::Unavailable);
    assert!(matches!(failure.cause, FailureCause::Original));
}

#[test]
fn related_row_failure_order_and_private_sql_message_remain_safe() {
    let (_root, catalog, package) = package_fixture();
    let resources = ContextMetadataResources::load(&catalog, &package).unwrap();
    let db = database();
    db.execute("update creators set family_name=null where id='first'", [])
        .unwrap();
    db.execute_batch("drop table identifiers").unwrap();
    let failure = resources.capture(&db, "paper").unwrap_err();
    assert_eq!(failure.error, PaperMetadataError::Unavailable);
    assert_eq!(
        data_cause(failure).resource_id,
        "literature.context-metadata.creators"
    );
    db.execute("update creators set family_name='' where id='first'", [])
        .unwrap();
    let failure = resources.capture(&db, "paper").unwrap_err();
    assert_eq!(failure.error, PaperMetadataError::Unavailable);
    let cause = data_cause(failure);
    assert_eq!(cause.resource_id, "literature.context-metadata.identifiers");
    assert!(cause.sqlite_code.is_some());
    let public = serde_json::to_string(&cause).unwrap();
    assert!(!public.contains("select kind"));
    assert!(!public.contains("no such table"));
    let db = database();
    db.execute("update identifiers set display_value=null where id='1'", [])
        .unwrap();
    let failure = resources.capture(&db, "paper").unwrap_err();
    assert_eq!(failure.error, PaperMetadataError::Unavailable);
    assert_eq!(
        data_cause(failure).resource_id,
        "literature.context-metadata.identifiers"
    );
}

#[test]
fn raw_header_witness_invalidates_equal_json_and_related_input_changes() {
    let (_root, catalog, package) = package_fixture();
    let resources = ContextMetadataResources::load(&catalog, &package).unwrap();
    let db = database();
    let raw = Arc::new(Mutex::new("{\"title\":\"same\"}".to_owned()));
    let returned = raw.clone();
    db.create_scalar_function("json_object", -1, FunctionFlags::default(), move |_| {
        Ok(returned.lock().unwrap().clone())
    })
    .unwrap();
    let first = resources.capture(&db, "paper").unwrap();
    *raw.lock().unwrap() = "{ \"title\": \"same\" }".into();
    let second = resources.capture(&db, "paper").unwrap();
    assert_eq!(first.input, second.input);
    assert_ne!(first, second);
    db.execute(
        "insert into item_creators values('paper','first','author',1)",
        [],
    )
    .unwrap();
    let duplicated = resources.capture(&db, "paper").unwrap();
    assert_eq!(duplicated.input.creators.len(), 3);
    assert_eq!(
        duplicated.input.creators[..2],
        [["  机构", "author"], ["  机构", "author"]]
    );
    assert_ne!(duplicated, second);
    db.execute(
        "update identifiers set display_value='changed' where id='1'",
        [],
    )
    .unwrap();
    assert_ne!(resources.capture(&db, "paper").unwrap(), duplicated);
}

#[test]
fn missing_or_changed_sql_never_becomes_an_empty_metadata_result() {
    let (root, catalog, package) = package_fixture();
    fs::remove_file(
        root.path()
            .join("literature/data/context-metadata-identifiers.v1.sql"),
    )
    .unwrap();
    assert_eq!(
        ContextMetadataResources::load(&catalog, &package)
            .err()
            .unwrap()
            .code,
        "runtime_plugin_resource_missing"
    );
    let (root, catalog, package) = package_fixture();
    fs::write(
        root.path()
            .join("literature/data/context-metadata-header.v1.sql"),
        "select 'private replacement'",
    )
    .unwrap();
    assert_eq!(
        ContextMetadataResources::load(&catalog, &package)
            .err()
            .unwrap()
            .code,
        "runtime_plugin_resource_changed"
    );
    let newly_discovered = catalog.discover().unwrap().packages.remove(0);
    assert_eq!(
        ContextMetadataResources::load(&catalog, &newly_discovered)
            .err()
            .unwrap()
            .code,
        "runtime_plugin_data_digest_mismatch"
    );
}
