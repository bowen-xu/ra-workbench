use super::*;
use std::{fs, path::Path};

fn package() -> (
    tempfile::TempDir,
    RuntimePluginCatalog,
    RuntimePluginPackage,
) {
    let directory = tempfile::tempdir().unwrap();
    let packages = directory.path().join("packages");
    let selected = packages.join("literature");
    fs::create_dir_all(selected.join("data")).unwrap();
    fs::write(
        selected.join("manifest.json"),
        r#"{"id":"literature_library","entry":"entry.js","styles":[]}"#,
    )
    .unwrap();
    fs::write(selected.join("entry.js"), "export {};\n").unwrap();
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/attachment-resources-v1");
    for spec in specs::SPECS {
        fs::copy(
            original.join(spec.file),
            selected.join("data").join(spec.file),
        )
        .unwrap();
    }
    let catalog = RuntimePluginCatalog::new(Some(packages));
    let package = catalog.discover().unwrap().packages.remove(0);
    (directory, catalog, package)
}

fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}

#[test]
fn exact_package_statements_reuse_duplicate_grant_and_reject_changed_bytes() {
    let (directory, catalog, package) = package();
    let resources = AttachmentResources::load(&catalog, &package).unwrap();
    assert_eq!(specs::SPECS.len(), 18);
    assert_eq!(resources.package_key(), package.package_key);
    let duplicate = resources.resource("duplicate-insert").unwrap();
    assert_eq!(
        duplicate.grant().id,
        "literature.catalog-save.duplicate-insert"
    );
    assert_eq!(
        duplicate.grant().sha256,
        "ebd0065d6ed08984a1bd96c02a4b8e4e20ef46a7a0416f6000a0da29ae1e167b"
    );
    let read = resources.resource("reading-state-get").unwrap();
    assert_eq!(read.grant().columns, [INTEGER, TEXT, OPTIONAL_TEXT]);
    let file = directory
        .path()
        .join("packages/literature/data/attachment-content-path.v1.sql");
    let mut bytes = fs::read(&file).unwrap();
    bytes.push(b'\n');
    fs::write(file, bytes).unwrap();
    assert!(AttachmentResources::load(&catalog, &package).is_err());
}

#[test]
fn docling_location_accepts_only_the_original_content_path_resource() {
    let (directory, catalog, _) = package();
    for spec in specs::SPECS {
        if spec.key != "content-path" {
            fs::remove_file(
                directory
                    .path()
                    .join("packages/literature/data")
                    .join(spec.file),
            )
            .unwrap();
        }
    }
    let package = catalog.discover().unwrap().packages.remove(0);
    let resources = AttachmentResources::load_content_path(&catalog, &package).unwrap();
    assert_eq!(resources.statements.len(), 1);
    let resource = resources.resource("content-path").unwrap();
    assert_eq!(resource.grant().access, DataAccess::Read);
    assert_eq!(resource.grant().sha256, specs::CONTENT_PATH.sha256);
    assert!(resources.resource("rename").is_err());
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table attachments(id,content_sha256);create table content_objects(sha256,relative_path);insert into attachments values('pdf','hash');insert into content_objects values('hash','nested/file.pdf');").unwrap();
    let before = db.total_changes();
    assert_eq!(
        resources
            .read(&db, "content-path", vec![text("pdf")])
            .unwrap()
            .rows,
        vec![vec![text("nested/file.pdf")]]
    );
    assert_eq!(db.total_changes(), before);
}

#[test]
fn reading_position_retains_i32_then_f64_bits_then_optional_text_without_transaction() {
    let (_directory, catalog, package) = package();
    let resources = AttachmentResources::load(&catalog, &package).unwrap();
    let db = Connection::open_in_memory().unwrap();
    // A projection fixture can hold incompatible cells without altering a
    // product migration; the original row.get failure order is the contract.
    db.execute_batch("create table reading_states(item_id,attachment_id,page_index,zoom,view_state_json);
        insert into reading_states values('paper','attachment',2147483648,'private bad zoom',null);").unwrap();
    let arguments = || vec![text("paper"), text("attachment")];
    let error = resources
        .read(&db, "reading-state-get", arguments())
        .unwrap_err();
    assert_eq!(error.safe_cause, "integral_value_out_of_range");
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private bad zoom")
    );
    for zoom in [f64::INFINITY, f64::NEG_INFINITY, -0.0, f64::from_bits(1)] {
        db.execute(
            "update reading_states set page_index=?1,zoom=?2",
            rusqlite::params![i32::MAX, zoom],
        )
        .unwrap();
        let original = db
            .query_row("select zoom from reading_states", [], |row| {
                row.get::<_, f64>(0)
            })
            .unwrap();
        let rows = resources
            .read(&db, "reading-state-get", arguments())
            .unwrap();
        assert_eq!(
            rows.rows,
            vec![vec![
                DataValue::Integer(i64::from(i32::MAX)),
                text(&format!("{:016x}", original.to_bits())),
                DataValue::Null
            ]]
        );
    }
    db.execute_batch("update reading_states set page_index=null,zoom='bad later cell';")
        .unwrap();
    assert_eq!(
        resources
            .read(&db, "reading-state-get", arguments())
            .unwrap_err()
            .safe_cause,
        "invalid_column_type"
    );
    assert!(
        resources
            .read(
                &db,
                "reading-state-get",
                vec![text("absent"), text("attachment")]
            )
            .unwrap()
            .rows
            .is_empty()
    );
    assert!(db.is_autocommit());
}

#[test]
fn original_index_sql_keeps_input_order_clamping_and_caller_rollback() {
    let (_directory, catalog, package) = package();
    let resources = AttachmentResources::load(&catalog, &package).unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table full_text_index_jobs(attachment_id primary key,state,page_count,error_message,updated_at);
        create table attachment_page_texts(attachment_id,page_index,page_text,indexed_at,primary key(attachment_id,page_index));
        create table attachments(id primary key,item_id);
        create table literature_search(item_id,full_text);
        insert into attachments values('attachment','paper');
        insert into literature_search values('paper','before');
        insert into full_text_index_jobs values('attachment','completed',1,null,'before');
        insert into attachment_page_texts values('attachment',5,'before','before');").unwrap();
    {
        let tx = db.transaction().unwrap();
        resources
            .write(
                &tx,
                "index-job-running",
                vec![text("attachment"), DataValue::Integer(2)],
            )
            .unwrap();
        resources
            .write(&tx, "index-pages-delete", vec![text("attachment")])
            .unwrap();
        resources
            .write(
                &tx,
                "index-page-insert",
                vec![
                    text("attachment"),
                    DataValue::Integer(-1),
                    text("first original input"),
                ],
            )
            .unwrap();
        let retained: String = tx
            .query_row(
                "select page_text from attachment_page_texts where page_index=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, "first original input");
        let failure = resources
            .write(
                &tx,
                "index-page-insert",
                vec![
                    text("attachment"),
                    DataValue::Integer(0),
                    text("second original input"),
                ],
            )
            .unwrap_err();
        assert_eq!(failure.sqlite_code.as_deref(), Some("ConstraintViolation"));
        assert!(!tx.is_autocommit());
        tx.rollback().unwrap();
    }
    let before: (String,i64,String) = db.query_row("select j.state,j.page_count,p.page_text from full_text_index_jobs j join attachment_page_texts p on p.attachment_id=j.attachment_id", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(before, ("completed".into(), 1, "before".into()));
    let tx = db.transaction().unwrap();
    resources
        .write(
            &tx,
            "index-job-running",
            vec![text("attachment"), DataValue::Integer(0)],
        )
        .unwrap();
    resources
        .write(&tx, "index-pages-delete", vec![text("attachment")])
        .unwrap();
    resources
        .write(&tx, "index-fts", vec![text("paper")])
        .unwrap();
    resources
        .write(&tx, "index-job-completed", vec![text("attachment")])
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.query_row(
            "select full_text from literature_search where item_id='paper'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        ""
    );
    assert_eq!(
        resources
            .read(&db, "is-indexed", vec![text("attachment")])
            .unwrap()
            .rows,
        vec![vec![DataValue::Integer(1)]]
    );
    assert!(db.is_autocommit());
}
