use super::*;
use research_workbench_native_sdk::data::{DataField, DataType};
use rusqlite::{Connection, TransactionBehavior, params};
use std::{sync::mpsc, time::Duration};

#[path = "fixture.rs"]
mod fixture;
use fixture::*;

#[path = "read_only/tests.rs"]
mod read_only_tests;

#[path = "write_tests.rs"]
mod write_tests;

#[path = "read_only/required_tests.rs"]
mod required_tests;

#[path = "read_only/ordered_tests.rs"]
mod ordered_tests;

#[path = "read_only/f64_bits_tests.rs"]
mod f64_bits_tests;

#[path = "connection_write_tests.rs"]
mod connection_write_tests;

#[path = "prepared_write/tests.rs"]
mod prepared_write_tests;

fn capture(
    db: &crate::data_interface::Database,
    resource: AcceptedDataResource,
) -> PreparedDataRead {
    let request = call(&resource, &["concept"]);
    let mut connection = db.lock().unwrap();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let prepared = PreparedDataRead::capture(&tx, resource, request).unwrap();
    tx.commit().unwrap();
    prepared
}

#[test]
fn package_statement_and_original_core_note_share_the_caller_transaction_and_reopen() {
    let fixture = DatabaseFixture::new();
    let package = PackageFixture::nal();
    let prepared = capture(&fixture.db, package.read());
    assert!(prepared.rows().rows.is_empty());
    let mutation = package.insert();
    let note_id;
    {
        let mut connection = fixture.db.lock().unwrap();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let checked = prepared.recheck_in(&tx).unwrap();
        note_id = note(&tx, "Atomic annotation note");
        assert_eq!(
            checked
                .execute(
                    &mutation,
                    &call(&mutation, &["annotation", "concept", &note_id])
                )
                .unwrap(),
            1
        );
        // Neither operation has committed independently of the caller.
        let observer = Connection::open(fixture.directory.path().join("fixture.sqlite3")).unwrap();
        assert_eq!(counts(&observer), (0, 0, 0));
        tx.commit().unwrap();
    }
    let fixture = fixture.reopen();
    let connection = fixture.db.lock().unwrap();
    assert_eq!(counts(&connection), (1, 1, 1));
    assert_eq!(
        connection
            .query_row(
                "select note_page_id from nal_annotations where id='annotation'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        note_id
    );
}

#[test]
fn plugin_trigger_failure_rolls_back_the_original_core_note_and_link_after_reopen() {
    let fixture = DatabaseFixture::new();
    let package = PackageFixture::nal();
    fixture
        .db
        .lock()
        .unwrap()
        .execute_batch(
            "create trigger simulate_link_failure before insert on nal_annotations \
         begin select raise(abort,'private-trigger-cause'); end;",
        )
        .unwrap();
    let prepared = capture(&fixture.db, package.read());
    let mutation = package.insert();
    {
        let mut connection = fixture.db.lock().unwrap();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let checked = prepared.recheck_in(&tx).unwrap();
        let note_id = note(&tx, "Must roll back");
        let error = checked
            .execute(
                &mutation,
                &call(&mutation, &["private-bound-id", "concept", &note_id]),
            )
            .unwrap_err();
        assert_eq!(error.code, "plugin_data_sql_failed");
        assert_eq!(error.phase, "execute");
        assert_eq!(
            error.sqlite_extended_code,
            Some(rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER)
        );
        let safe = serde_json::to_string(&error).unwrap();
        for private in [
            INSERT_SQL,
            "private-trigger-cause",
            "private-bound-id",
            &note_id,
        ] {
            assert!(!safe.contains(private), "diagnostic leaked {private}");
            assert!(!error.to_string().contains(private));
        }
        assert_eq!(error.resource_id, mutation.grant().id);
        assert_eq!(error.package_key, mutation.package_key());
        tx.rollback().unwrap();
    }
    let fixture = fixture.reopen();
    assert_eq!(counts(&fixture.db.lock().unwrap()), (0, 0, 0));
}

#[test]
fn an_absence_phantom_is_rejected_even_when_the_compatibility_read_fence_is_unchanged() {
    let fixture = DatabaseFixture::new();
    let package = PackageFixture::nal();
    let before = fixture.db.lock().unwrap().fence();
    let prepared = capture(&fixture.db, package.read());
    {
        let mut connection = fixture.db.lock().unwrap();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let note_id = note(&tx, "Concurrent original writer");
        tx.execute(INSERT_SQL, params!["concurrent", "concept", note_id])
            .unwrap();
        tx.commit().unwrap();
    }
    let mut connection = fixture.db.lock().unwrap();
    assert_eq!(connection.fence(), before);
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let error = prepared.recheck_in(&tx).err().unwrap();
    assert_eq!(error.code, "plugin_data_snapshot_changed");
    assert_eq!(error.phase, "recheck");
    // A typed caller stops before creating its dependent Core document.
    assert_eq!(counts(&tx), (1, 1, 1));
    tx.rollback().unwrap();
}

#[test]
fn capture_reads_the_callers_uncommitted_rows_without_opening_another_connection() {
    let fixture = DatabaseFixture::new();
    let package = PackageFixture::new(&[(
        "concept.sql",
        "select id from concepts where id=?1 order by id",
    )]);
    let resource = package.resource("concept.sql", DataAccess::Read, vec![text()], vec![text()]);
    let mut connection = fixture.db.lock().unwrap();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    tx.execute(
        "insert into concepts(id,label,explanation,created_at,updated_at) values('uncommitted','Pending','','now','now')", [],
    ).unwrap();
    let prepared =
        PreparedDataRead::capture(&tx, resource.clone(), call(&resource, &["uncommitted"]))
            .unwrap();
    assert_eq!(
        prepared.rows().rows,
        vec![vec![DataValue::Text("uncommitted".into())]]
    );
    let observer = Connection::open(fixture.directory.path().join("fixture.sqlite3")).unwrap();
    assert_eq!(
        observer
            .query_row(
                "select count(*) from concepts where id='uncommitted'",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    tx.rollback().unwrap();
    // The exported data is owned, even after its original transaction ends.
    assert_eq!(prepared.rows().rows.len(), 1);
}

#[test]
fn captured_rows_do_not_retain_admission_or_block_an_independent_core_note_save() {
    let fixture = DatabaseFixture::new();
    let package = PackageFixture::nal();
    let prepared = capture(&fixture.db, package.read());
    let db = fixture.db.clone();
    let (sender, receiver) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let mut connection = db.lock().unwrap();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let note_id = note(&tx, "Core remains available while native calculates");
        tx.commit().unwrap();
        sender.send(note_id).unwrap();
    });
    let note_id = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("owned capture must release the Core guard before native work");
    writer.join().unwrap();
    assert!(!note_id.is_empty());
    assert!(prepared.rows().rows.is_empty());
    assert_eq!(counts(&fixture.db.lock().unwrap()), (1, 1, 0));
}

#[test]
fn rejects_bad_call_identity_argument_shape_and_access_before_any_sql_write() {
    let fixture = DatabaseFixture::new();
    let package = PackageFixture::nal();
    let resource = package.read();
    let valid = call(&resource, &["concept"]);
    let mut invalid_calls = vec![
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
            arguments: vec![DataValue::Integer(1)],
            ..valid.clone()
        },
        DataCall {
            arguments: vec![DataValue::Null],
            ..valid
        },
    ];
    let mut connection = fixture.db.lock().unwrap();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    for request in invalid_calls.drain(..) {
        let error = PreparedDataRead::capture(&tx, resource.clone(), request)
            .err()
            .unwrap();
        assert_eq!(error.code, "plugin_data_call_invalid");
    }
    let mutation = package.insert();
    let error = PreparedDataRead::capture(
        &tx,
        mutation.clone(),
        call(&mutation, &["id", "concept", "note"]),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "plugin_data_call_invalid");
    let prepared =
        PreparedDataRead::capture(&tx, resource.clone(), call(&resource, &["concept"])).unwrap();
    let checked = prepared.recheck_in(&tx).unwrap();
    assert_eq!(
        checked
            .execute(&resource, &call(&resource, &["concept"]))
            .unwrap_err()
            .code,
        "plugin_data_call_invalid"
    );
    assert_eq!(counts(&tx), (0, 0, 0));
    tx.rollback().unwrap();
}

#[test]
fn rejects_statement_column_parameter_and_readonly_mismatches_without_stepping() {
    let fixture = DatabaseFixture::new();
    let package = PackageFixture::new(&[
        ("annotations.sql", READ_SQL),
        ("columns.sql", "select id,label from concepts where id=?1"),
        (
            "parameters.sql",
            "select id from concepts where id=?1 or id=?2",
        ),
        ("write.sql", "delete from concepts where id=?1"),
        (
            "returning.sql",
            "delete from concepts where id=?1 returning id",
        ),
    ]);
    let prepared = capture(&fixture.db, package.read());
    let mut connection = fixture.db.lock().unwrap();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    for name in [
        "columns.sql",
        "parameters.sql",
        "write.sql",
        "returning.sql",
    ] {
        let resource = package.resource(name, DataAccess::Read, vec![text()], vec![text()]);
        let error = PreparedDataRead::capture(&tx, resource.clone(), call(&resource, &["concept"]))
            .err()
            .unwrap();
        assert_eq!(error.code, "plugin_data_statement_incompatible", "{name}");
        assert_eq!(error.phase, "prepare");
    }
    // A write grant with RETURNING is incompatible before it can delete a row.
    let resource = package.resource("returning.sql", DataAccess::Write, vec![text()], vec![]);
    let error = prepared
        .recheck_in(&tx)
        .unwrap()
        .execute(&resource, &call(&resource, &["concept"]))
        .unwrap_err();
    assert_eq!(error.code, "plugin_data_statement_incompatible");
    assert_eq!(
        tx.query_row("select count(*) from concepts", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    tx.rollback().unwrap();
}

#[test]
fn result_types_and_cross_package_writes_are_rejected_with_safe_diagnostics() {
    let fixture = DatabaseFixture::new();
    let package = PackageFixture::new(&[
        ("typed.sql", "select 7 from concepts where id=?1"),
        (
            "missing.sql",
            "select private_synthetic_column from private_missing_table where id=?1",
        ),
    ]);
    let original = PackageFixture::nal();
    let foreign = PackageFixture::new(&[
        ("annotations.sql", READ_SQL),
        ("insert.sql", INSERT_SQL),
        ("version.sql", "select 1"),
    ]);
    let prepared = capture(&fixture.db, original.read());
    let mut connection = fixture.db.lock().unwrap();
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let typed = package.resource("typed.sql", DataAccess::Read, vec![text()], vec![text()]);
    let error = PreparedDataRead::capture(&tx, typed.clone(), call(&typed, &["concept"]))
        .err()
        .unwrap();
    assert_eq!(error.code, "plugin_data_result_invalid");
    assert_eq!(error.phase, "decode");
    let missing = package.resource(
        "missing.sql",
        DataAccess::Read,
        vec![text()],
        vec![DataField {
            data_type: DataType::Integer,
            nullable: false,
        }],
    );
    let error = PreparedDataRead::capture(
        &tx,
        missing.clone(),
        call(&missing, &["private-bound-value"]),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "plugin_data_sql_failed");
    assert_eq!(error.phase, "prepare");
    assert!(error.sqlite_code.is_some());
    let wire = serde_json::to_string(&error).unwrap();
    for private in [
        "private_synthetic_column",
        "private_missing_table",
        "private-bound-value",
        missing.sql(),
    ] {
        assert!(!wire.contains(private));
    }
    let mutation = foreign.insert();
    let error = prepared
        .recheck_in(&tx)
        .unwrap()
        .execute(&mutation, &call(&mutation, &["id", "concept", "note"]))
        .unwrap_err();
    assert_eq!(error.code, "plugin_data_binding_mismatch");
    assert_eq!(error.phase, "authorize");
    assert_eq!(counts(&tx), (0, 0, 0));
    tx.rollback().unwrap();
}

#[test]
fn syntax_error_preserves_sqlite_codes_without_disclosing_input_or_ending_the_caller_transaction() {
    let fixture = DatabaseFixture::new();
    let sql = "select id from concepts where id=?1 private_syntax_token";
    let package = PackageFixture::new(&[("syntax.sql", sql)]);
    let resource = package.resource("syntax.sql", DataAccess::Read, vec![text()], vec![text()]);
    {
        let mut connection = fixture.db.lock().unwrap();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let note_id = note(&tx, "Original caller owns rollback");
        // This real driver error carries SQL, an error message and an offset;
        // the adapter must preserve only its safe SQLite codes.
        assert!(matches!(
            tx.prepare(sql).err().unwrap(),
            rusqlite::Error::SqlInputError { .. }
        ));
        let error = PreparedDataRead::capture(
            &tx,
            resource.clone(),
            call(&resource, &["private-syntax-bound-value"]),
        )
        .err()
        .unwrap();
        let expected_code = format!(
            "{:?}",
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR).code
        );
        assert_eq!(error.code, "plugin_data_sql_failed");
        assert_eq!(error.phase, "prepare");
        assert_eq!(error.sqlite_code.as_deref(), Some(expected_code.as_str()));
        assert_eq!(
            error.sqlite_extended_code,
            Some(rusqlite::ffi::SQLITE_ERROR)
        );
        assert_eq!(error.safe_cause, expected_code);
        assert_eq!(error.resource_id, resource.grant().id);
        assert_eq!(error.package_key, resource.package_key());
        let wire = serde_json::to_string(&error).unwrap();
        for private in [
            sql,
            "private_syntax_token",
            "private-syntax-bound-value",
            &note_id,
        ] {
            assert!(!wire.contains(private));
            assert!(!error.to_string().contains(private));
            assert!(!format!("{error:?}").contains(private));
        }
        assert!(
            error
                .original_sql_failure
                .as_ref()
                .unwrap()
                .message()
                .contains("private_syntax_token")
        );
        assert!(!tx.is_autocommit());
        assert_eq!(counts(&tx), (1, 1, 0));
        tx.rollback().unwrap();
    }
    let fixture = fixture.reopen();
    assert_eq!(counts(&fixture.db.lock().unwrap()), (0, 0, 0));
}
