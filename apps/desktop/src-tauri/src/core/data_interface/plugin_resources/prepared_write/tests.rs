use super::*;

#[test]
fn preparing_all_collectors_never_steps_an_earlier_write_before_a_late_prepare_error() {
    let package = PackageFixture::new(&[
        ("first.sql", "insert into records(id) values(?1)"),
        ("last.sql", "insert into missing_records(id) values(?1)"),
    ]);
    let first = package.resource("first.sql", DataAccess::Write, vec![text()], vec![]);
    let last = package.resource("last.sql", DataAccess::Write, vec![text()], vec![]);
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table records(id text primary key)")
        .unwrap();
    let tx = db.transaction().unwrap();
    let prepared = PreparedDataWrite::prepare_in(&tx, &first, call(&first, &["first"])).unwrap();
    let error = PreparedDataWrite::prepare_in(&tx, &last, call(&last, &["last"]))
        .err()
        .unwrap();
    assert_eq!(error.phase, "prepare");
    assert_eq!(error.sqlite_extended_code, Some(1));
    assert!(!error.safe_cause.contains("missing_records"));
    assert_eq!(
        tx.query_row("select count(*) from records", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(!tx.is_autocommit());
    drop(prepared);
    tx.rollback().unwrap();
}

#[test]
fn prepared_calls_keep_fixed_arguments_across_rounds_and_stop_if_the_original_transaction_ends() {
    let package =
        PackageFixture::new(&[("insert.sql", "insert or ignore into records(id) values(?1)")]);
    let resource = package.resource("insert.sql", DataAccess::Write, vec![text()], vec![]);
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table records(id text primary key)")
        .unwrap();
    let tx = db.transaction().unwrap();
    let mut prepared =
        PreparedDataWrite::prepare_in(&tx, &resource, call(&resource, &["original\0identity"]))
            .unwrap();
    assert_eq!(prepared.execute().unwrap(), 1);
    assert_eq!(prepared.execute().unwrap(), 0);
    assert_eq!(
        tx.query_row("select id from records", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "original\0identity"
    );
    tx.execute_batch("ROLLBACK").unwrap();
    let error = prepared.execute().unwrap_err();
    assert_eq!(error.phase, "validate");
    assert_eq!(error.safe_cause, "caller_transaction_missing");
    assert!(tx.is_autocommit());
    assert_eq!(
        tx.query_row("select count(*) from records", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn preparation_preserves_access_identity_parameter_and_statement_shape_gates() {
    let package = PackageFixture::new(&[
        ("insert.sql", "insert into records(id) values(?1)"),
        ("read.sql", "select id from records"),
        (
            "returning.sql",
            "insert into records(id) values(?1) returning id",
        ),
    ]);
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table records(id text primary key)")
        .unwrap();
    let tx = db.transaction().unwrap();
    let read = package.resource("read.sql", DataAccess::Read, vec![], vec![text()]);
    assert_eq!(
        PreparedDataWrite::prepare_in(&tx, &read, call(&read, &[]))
            .err()
            .unwrap()
            .safe_cause,
        "resource_access_mismatch"
    );
    let resource = package.resource("insert.sql", DataAccess::Write, vec![text()], vec![]);
    let valid = call(&resource, &["not-written"]);
    for invalid in [
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
    ] {
        assert_eq!(
            PreparedDataWrite::prepare_in(&tx, &resource, invalid)
                .err()
                .unwrap()
                .code,
            "plugin_data_call_invalid"
        );
    }
    for (name, args) in [("read.sql", vec![]), ("returning.sql", vec!["not-written"])] {
        let fields = if args.is_empty() {
            vec![]
        } else {
            vec![text()]
        };
        let resource = package.resource(name, DataAccess::Write, fields, vec![]);
        assert_eq!(
            PreparedDataWrite::prepare_in(&tx, &resource, call(&resource, &args))
                .err()
                .unwrap()
                .code,
            "plugin_data_statement_incompatible"
        );
    }
    assert_eq!(
        tx.query_row("select count(*) from records", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn prepared_write_and_core_note_commit_or_roll_back_in_the_same_sql_log_owner() {
    for commit in [false, true] {
        let fixture = DatabaseFixture::new();
        let package = PackageFixture::nal();
        let resource = package.insert();
        let mut db = fixture.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        let note_id = note(&tx, "Original Core note owner");
        let mut prepared = PreparedDataWrite::prepare_in(
            &tx,
            &resource,
            call(&resource, &["annotation", "concept", &note_id]),
        )
        .unwrap();
        assert_eq!(prepared.execute().unwrap(), 1);
        assert_eq!(counts(&tx), (1, 1, 1));
        let observer = Connection::open(fixture.directory.path().join("fixture.sqlite3")).unwrap();
        assert_eq!(counts(&observer), (0, 0, 0));
        drop(observer);
        drop(prepared);
        if commit {
            tx.commit().unwrap();
        } else {
            tx.rollback().unwrap();
        }
        drop(db);
        let fixture = fixture.reopen();
        assert_eq!(
            counts(&fixture.db.lock().unwrap()),
            if commit { (1, 1, 1) } else { (0, 0, 0) }
        );
    }
}
