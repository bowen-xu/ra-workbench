use super::*;

#[test]
fn fixed_write_and_core_note_share_the_original_deferred_transaction_without_a_read_proof() {
    let fixture = DatabaseFixture::new();
    // This accepted package has no read resource from which to fabricate proof.
    let package = PackageFixture::new(&[("insert.sql", INSERT_SQL)]);
    let mutation = package.insert();
    let mut connection = fixture.db.lock().unwrap();
    let tx = connection.transaction().unwrap();
    let note_id = note(&tx, "Caller-owned rollback");
    assert_eq!(
        write_in(
            &tx,
            &mutation,
            &call(&mutation, &["annotation", "concept", &note_id]),
        )
        .unwrap(),
        1
    );
    assert!(!tx.is_autocommit());
    assert_eq!(counts(&tx), (1, 1, 1));
    let observer = Connection::open(fixture.directory.path().join("fixture.sqlite3")).unwrap();
    assert_eq!(counts(&observer), (0, 0, 0));
    tx.rollback().unwrap();
    assert_eq!(counts(&connection), (0, 0, 0));
}

#[test]
fn fixed_write_rejects_an_already_closed_caller_transaction_without_starting_another() {
    let package = PackageFixture::new(&[("insert.sql", "insert into records(value) values(?1)")]);
    let mutation = package.resource("insert.sql", DataAccess::Write, vec![text()], vec![]);
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("create table records(value text)")
        .unwrap();
    let tx = connection.transaction().unwrap();
    tx.execute_batch("rollback").unwrap();
    let error = write_in(&tx, &mutation, &call(&mutation, &["not-written"])).unwrap_err();
    assert_eq!(error.safe_cause, "caller_transaction_missing");
    assert!(tx.is_autocommit());
    assert_eq!(
        tx.query_row("select count(*) from records", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn fixed_write_preserves_access_parameter_and_statement_rejections_before_stepping() {
    let package = PackageFixture::new(&[
        ("insert.sql", "insert into records(value) values(?1)"),
        ("select.sql", "select value from records"),
        (
            "returning.sql",
            "insert into records(value) values(?1) returning value",
        ),
    ]);
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("create table records(value text)")
        .unwrap();
    let tx = connection.transaction().unwrap();
    let read = package.resource("select.sql", DataAccess::Read, vec![], vec![text()]);
    assert_eq!(
        write_in(&tx, &read, &call(&read, &[]))
            .unwrap_err()
            .safe_cause,
        "resource_access_mismatch"
    );
    let mutation = package.resource("insert.sql", DataAccess::Write, vec![text()], vec![]);
    let valid = call(&mutation, &["not-written"]);
    for request in [
        DataCall {
            id: "literature.other".into(),
            ..valid.clone()
        },
        DataCall {
            version: 2,
            ..valid.clone()
        },
        DataCall {
            arguments: vec![],
            ..valid.clone()
        },
        DataCall {
            arguments: vec![DataValue::Null],
            ..valid.clone()
        },
        DataCall {
            arguments: vec![DataValue::Integer(1)],
            ..valid.clone()
        },
    ] {
        assert_eq!(
            write_in(&tx, &mutation, &request).unwrap_err().code,
            "plugin_data_call_invalid"
        );
    }
    for name in ["select.sql", "returning.sql"] {
        let parameters = if name == "select.sql" {
            vec![]
        } else {
            vec![text()]
        };
        let resource = package.resource(name, DataAccess::Write, parameters, vec![]);
        let request = if name == "select.sql" {
            call(&resource, &[])
        } else {
            call(&resource, &["not-written"])
        };
        assert_eq!(
            write_in(&tx, &resource, &request).unwrap_err().code,
            "plugin_data_statement_incompatible"
        );
    }
    assert_eq!(
        tx.query_row("select count(*) from records", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert!(!tx.is_autocommit());
    tx.rollback().unwrap();
}

#[test]
fn failed_fixed_write_preserves_safe_sqlite_codes_and_the_callers_uncommitted_work() {
    let package = PackageFixture::new(&[("insert.sql", "insert into records(value) values(?1)")]);
    let mutation = package.resource("insert.sql", DataAccess::Write, vec![text()], vec![]);
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("create table records(value text unique)")
        .unwrap();
    let tx = connection.transaction().unwrap();
    tx.execute("insert into records(value) values(?1)", ["private-body"])
        .unwrap();
    let error = write_in(&tx, &mutation, &call(&mutation, &["private-body"])).unwrap_err();
    assert_eq!(error.code, "plugin_data_sql_failed");
    assert_eq!(error.phase, "execute");
    assert!(error.sqlite_code.is_some() && error.sqlite_extended_code.is_some());
    assert_eq!(error.package_key, mutation.package_key());
    assert_eq!(error.resource_id, mutation.grant().id);
    let encoded = serde_json::to_string(&error).unwrap();
    for private in ["private-body", "insert into", "records(value)"] {
        assert!(!encoded.contains(private));
    }
    assert_eq!(
        tx.query_row("select count(*) from records", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert!(!tx.is_autocommit());
    tx.rollback().unwrap();
    assert_eq!(
        connection
            .query_row("select count(*) from records", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
}
