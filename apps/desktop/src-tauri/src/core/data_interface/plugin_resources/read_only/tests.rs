use super::*;

fn integer(nullable: bool) -> DataField {
    DataField {
        data_type: DataType::Integer,
        nullable,
    }
}

fn resource(sql: &str, fields: Vec<DataField>) -> AcceptedDataResource {
    PackageFixture::new(&[("read.sql", sql)]).resource("read.sql", DataAccess::Read, vec![], fields)
}

fn read(
    db: &Connection,
    resource: &AcceptedDataResource,
    policy: RowPolicy,
    codecs: &[ColumnCodec],
) -> Result<DataRows> {
    read_on_connection(db, resource, &call(resource, &[]), policy, codecs)
}

#[test]
fn connection_reads_neither_start_nor_finish_the_callers_transaction() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table items(value text); insert into items values('committed');")
        .unwrap();
    let resource = resource("select value from items order by rowid", vec![text()]);
    assert_eq!(
        read(&db, &resource, RowPolicy::All, &[ColumnCodec::String])
            .unwrap()
            .rows,
        vec![vec![DataValue::Text("committed".into())]]
    );
    assert!(db.is_autocommit());
    db.execute_batch("begin immediate; insert into items values('uncommitted');")
        .unwrap();
    let rows = read(&db, &resource, RowPolicy::All, &[ColumnCodec::String]).unwrap();
    assert_eq!(rows.rows[1], vec![DataValue::Text("uncommitted".into())]);
    assert!(!db.is_autocommit());
    db.execute_batch("rollback").unwrap();
    assert_eq!(
        read(&db, &resource, RowPolicy::All, &[ColumnCodec::String])
            .unwrap()
            .rows
            .len(),
        1
    );
}

#[test]
fn original_transactional_read_keeps_full_width_integer_values() {
    let mut db = Connection::open_in_memory().unwrap();
    let data = resource(
        "select -9223372036854775808,null,9223372036854775807",
        vec![integer(false), integer(true), integer(false)],
    );
    let tx = db.transaction().unwrap();
    let result = read_in(&tx, &data, &call(&data, &[])).unwrap();
    assert_eq!(
        result.rows,
        vec![vec![
            DataValue::Integer(i64::MIN),
            DataValue::Null,
            DataValue::Integer(i64::MAX),
        ]]
    );
    assert!(!tx.is_autocommit());
    tx.rollback().unwrap();
    assert!(db.is_autocommit());
}

#[test]
fn first_optional_does_not_step_a_later_row_and_absence_is_empty() {
    let db = Connection::open_in_memory().unwrap();
    let data = resource(
        "select 7 union all select abs(-9223372036854775808)",
        vec![integer(false)],
    );
    assert_eq!(
        read(&db, &data, RowPolicy::FirstOptional, &[ColumnCodec::U64])
            .unwrap()
            .rows,
        vec![vec![DataValue::Integer(7)]]
    );
    let failure = read(&db, &data, RowPolicy::All, &[ColumnCodec::U64]).unwrap_err();
    assert_eq!(failure.code, "plugin_data_sql_failed");
    assert!(failure.sqlite_code.is_some());
    let absent = resource(
        "select abs(-9223372036854775808) where 0",
        vec![integer(false)],
    );
    assert!(
        read(&db, &absent, RowPolicy::FirstOptional, &[ColumnCodec::U64])
            .unwrap()
            .rows
            .is_empty()
    );
    assert!(db.is_autocommit());
}

#[test]
fn codecs_preserve_rusqlite_integer_options_and_nonzero_boolean_rules() {
    let db = Connection::open_in_memory().unwrap();
    let data = resource(
        "select 'raw text',null,2147483647,null,9223372036854775807,7,-2,0",
        vec![
            text(),
            DataField {
                nullable: true,
                ..text()
            },
            integer(true),
            integer(true),
            integer(false),
            integer(false),
            integer(false),
            integer(false),
        ],
    );
    let codecs = [
        ColumnCodec::String,
        ColumnCodec::OptionalString,
        ColumnCodec::OptionalI32,
        ColumnCodec::OptionalI32,
        ColumnCodec::U64,
        ColumnCodec::Bool,
        ColumnCodec::Bool,
        ColumnCodec::Bool,
    ];
    assert_eq!(
        read(&db, &data, RowPolicy::All, &codecs).unwrap().rows,
        vec![vec![
            DataValue::Text("raw text".into()),
            DataValue::Null,
            DataValue::Integer(i64::from(i32::MAX)),
            DataValue::Null,
            DataValue::Integer(i64::MAX),
            DataValue::Integer(1),
            DataValue::Integer(1),
            DataValue::Integer(0),
        ]]
    );
    let optional_values = resource(
        "select ' raw ',-2147483648,0",
        vec![
            DataField {
                nullable: true,
                ..text()
            },
            integer(true),
            integer(false),
        ],
    );
    assert_eq!(
        read(
            &db,
            &optional_values,
            RowPolicy::All,
            &[
                ColumnCodec::OptionalString,
                ColumnCodec::OptionalI32,
                ColumnCodec::U64
            ],
        )
        .unwrap()
        .rows,
        vec![vec![
            DataValue::Text(" raw ".into()),
            DataValue::Integer(i64::from(i32::MIN)),
            DataValue::Integer(0),
        ]]
    );
    for (sql, codec, field) in [
        ("select 2147483648", ColumnCodec::OptionalI32, integer(true)),
        (
            "select -2147483649",
            ColumnCodec::OptionalI32,
            integer(true),
        ),
        ("select -1", ColumnCodec::U64, integer(false)),
        (
            "select 9223372036854775808",
            ColumnCodec::U64,
            integer(false),
        ),
        ("select 1.0", ColumnCodec::OptionalI32, integer(true)),
        ("select null", ColumnCodec::U64, integer(false)),
        ("select '1'", ColumnCodec::OptionalI32, integer(true)),
        ("select null", ColumnCodec::String, text()),
        ("select 1.0", ColumnCodec::Bool, integer(false)),
        ("select '1'", ColumnCodec::Bool, integer(false)),
        ("select null", ColumnCodec::Bool, integer(false)),
    ] {
        let error = read(&db, &resource(sql, vec![field]), RowPolicy::All, &[codec]).unwrap_err();
        assert_eq!(error.code, "plugin_data_sql_failed");
        assert_eq!(error.phase, "read");
    }
}

#[test]
fn all_keeps_original_row_order_and_decodes_columns_before_later_rows() {
    let db = Connection::open_in_memory().unwrap();
    let ordered = resource(
        "select 4 union all select 2 union all select 9",
        vec![integer(false)],
    );
    assert_eq!(
        read(&db, &ordered, RowPolicy::All, &[ColumnCodec::U64])
            .unwrap()
            .rows,
        vec![
            vec![DataValue::Integer(4)],
            vec![DataValue::Integer(2)],
            vec![DataValue::Integer(9)]
        ]
    );
    // A first-column i32 overflow must win over the second-column NULL, and
    // decoding that row must win over stepping the later SQL overflow row.
    let bad = resource(
        "select 2147483648,null union all select abs(-9223372036854775808),'later private body'",
        vec![integer(true), text()],
    );
    let error = read(
        &db,
        &bad,
        RowPolicy::All,
        &[ColumnCodec::OptionalI32, ColumnCodec::String],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "integral_value_out_of_range");
    assert!(error.sqlite_code.is_none());
    let second = resource("select 1,null", vec![integer(true), text()]);
    let error = read(
        &db,
        &second,
        RowPolicy::All,
        &[ColumnCodec::OptionalI32, ColumnCodec::String],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "invalid_column_type");
}

#[test]
fn read_only_uses_the_original_grant_statement_and_safe_error_boundaries() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table items(value text); insert into items values('private body');")
        .unwrap();
    let package = PackageFixture::new(&[("write.sql", "delete from items")]);
    let write = package.resource("write.sql", DataAccess::Write, vec![], vec![]);
    let error = read(&db, &write, RowPolicy::All, &[]).unwrap_err();
    assert_eq!(error.safe_cause, "resource_access_mismatch");
    let disguised = resource("delete from items returning value", vec![text()]);
    let error = read(&db, &disguised, RowPolicy::All, &[ColumnCodec::String]).unwrap_err();
    assert_eq!(error.code, "plugin_data_statement_incompatible");
    assert_eq!(
        db.query_row("select count(*) from items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let good = resource("select value from items", vec![text()]);
    let error = read(&db, &good, RowPolicy::All, &[ColumnCodec::U64]).unwrap_err();
    assert_eq!(error.safe_cause, "column_codec_mismatch");
    let invalid = resource(
        "select PRIVATE_SQL_BODY from missing_private_table",
        vec![text()],
    );
    let error = read(&db, &invalid, RowPolicy::All, &[ColumnCodec::String]).unwrap_err();
    assert_eq!(error.code, "plugin_data_sql_failed");
    assert_eq!(error.phase, "prepare");
    assert!(error.sqlite_code.is_some() && error.sqlite_extended_code.is_some());
    let json = serde_json::to_string(&error).unwrap();
    for private in ["PRIVATE_SQL_BODY", "missing_private_table", "private body"] {
        assert!(!json.contains(private));
    }
    let invalid_text = resource("select cast(x'80' as text)", vec![text()]);
    let error = read(&db, &invalid_text, RowPolicy::All, &[ColumnCodec::String]).unwrap_err();
    assert_eq!(error.safe_cause, "column_conversion_failed");
    assert!(error.sqlite_code.is_none() && error.sqlite_extended_code.is_none());
    assert!(db.is_autocommit());
}
