use super::*;

fn resource(sql: &str) -> AcceptedDataResource {
    PackageFixture::new(&[("read.sql", sql)]).resource(
        "read.sql",
        DataAccess::Read,
        vec![],
        vec![DataField {
            data_type: DataType::Integer,
            nullable: false,
        }],
    )
}

fn read(db: &Connection, resource: &AcceptedDataResource, policy: RowPolicy) -> Result<DataRows> {
    read_on_connection(
        db,
        resource,
        &call(resource, &[]),
        policy,
        &[ColumnCodec::I64],
    )
}

#[test]
fn required_first_preserves_query_row_and_does_not_step_later_rows() {
    let db = Connection::open_in_memory().unwrap();
    let data = resource("select -7 union all select abs(-9223372036854775808)");
    assert_eq!(
        read(&db, &data, RowPolicy::FirstRequired).unwrap().rows,
        vec![vec![DataValue::Integer(-7)]]
    );
    assert!(read(&db, &data, RowPolicy::All).is_err());
    assert!(db.is_autocommit());
}

#[test]
fn required_absence_is_original_no_rows_error_while_other_policies_stay_empty() {
    let db = Connection::open_in_memory().unwrap();
    let data = resource("select 1 where 0");
    let failure = read(&db, &data, RowPolicy::FirstRequired).unwrap_err();
    assert_eq!(failure.code, "plugin_data_sql_failed");
    assert_eq!(failure.phase, "read");
    assert_eq!(failure.safe_cause, "query_returned_no_rows");
    assert!(
        read(&db, &data, RowPolicy::FirstOptional)
            .unwrap()
            .rows
            .is_empty()
    );
    assert!(read(&db, &data, RowPolicy::All).unwrap().rows.is_empty());
}

#[test]
fn i64_codec_keeps_full_width_and_rejects_non_integer_storage_without_coercion() {
    let db = Connection::open_in_memory().unwrap();
    for value in ["-9223372036854775808", "9223372036854775807"] {
        let data = resource(&format!("select {value}"));
        assert_eq!(
            read(&db, &data, RowPolicy::FirstRequired).unwrap().rows,
            vec![vec![DataValue::Integer(value.parse().unwrap())]]
        );
    }
    for value in ["null", "1.5", "'12'", "x'01'"] {
        let data = resource(&format!("select {value}"));
        assert!(read(&db, &data, RowPolicy::FirstRequired).is_err());
    }
}

#[test]
fn nonnullable_i32_keeps_original_width_and_rejects_optional_or_coerced_values() {
    let db = Connection::open_in_memory().unwrap();
    for value in ["-2147483648", "2147483647"] {
        let sql = format!("select {value}");
        let original = db.query_row(&sql, [], |row| row.get::<_, i32>(0)).unwrap();
        let data = resource(&sql);
        let rows = read_on_connection(
            &db,
            &data,
            &call(&data, &[]),
            RowPolicy::FirstRequired,
            &[ColumnCodec::I32],
        )
        .unwrap();
        assert_eq!(
            rows.rows,
            vec![vec![DataValue::Integer(i64::from(original))]]
        );
    }
    for (value, category) in [
        ("-2147483649", "integral_value_out_of_range"),
        ("2147483648", "integral_value_out_of_range"),
        ("null", "invalid_column_type"),
        ("1.5", "invalid_column_type"),
        ("'12'", "invalid_column_type"),
        ("x'01'", "invalid_column_type"),
    ] {
        let sql = format!("select {value}");
        assert!(db.query_row(&sql, [], |row| row.get::<_, i32>(0)).is_err());
        let data = resource(&sql);
        let error = read_on_connection(
            &db,
            &data,
            &call(&data, &[]),
            RowPolicy::FirstRequired,
            &[ColumnCodec::I32],
        )
        .unwrap_err();
        assert_eq!(error.phase, "read");
        assert_eq!(error.safe_cause, category);
    }
    assert!(db.is_autocommit());
}

#[test]
fn i32_failure_precedes_later_columns_and_does_not_accept_a_nullable_grant() {
    let db = Connection::open_in_memory().unwrap();
    let sql = "select 2147483648,'private invalid zoom'";
    let data = PackageFixture::new(&[("read.sql", sql)]).resource(
        "read.sql",
        DataAccess::Read,
        vec![],
        vec![
            DataField {
                data_type: DataType::Integer,
                nullable: false,
            },
            text(),
        ],
    );
    let error = read_on_connection(
        &db,
        &data,
        &call(&data, &[]),
        RowPolicy::FirstOptional,
        &[ColumnCodec::I32, ColumnCodec::F64Bits],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "integral_value_out_of_range");
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private invalid zoom")
    );
    let nullable = PackageFixture::new(&[("read.sql", "select 1")]).resource(
        "read.sql",
        DataAccess::Read,
        vec![],
        vec![DataField {
            data_type: DataType::Integer,
            nullable: true,
        }],
    );
    let error = read_on_connection(
        &db,
        &nullable,
        &call(&nullable, &[]),
        RowPolicy::FirstOptional,
        &[ColumnCodec::I32],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "column_codec_mismatch");
}
