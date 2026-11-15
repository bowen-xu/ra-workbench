use super::*;

fn resource(sql: &str, columns: Vec<DataField>) -> AcceptedDataResource {
    PackageFixture::new(&[("read.sql", sql)]).resource(
        "read.sql",
        DataAccess::Read,
        vec![],
        columns,
    )
}

fn read(db: &Connection, resource: &AcceptedDataResource) -> Result<DataRows> {
    read_on_connection(
        db,
        resource,
        &call(resource, &[]),
        RowPolicy::All,
        &[ColumnCodec::F64Bits],
    )
}

#[test]
fn float_bits_keep_original_numeric_conversion_and_private_text_wire_exactly() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table numbers(value);").unwrap();
    for value in [
        f64::from_bits(0x3fef_ffff_ffff_ffff),
        f64::from_bits(0x0b77_7f24_5544_ff83),
        f64::from_bits(1),
        f64::MIN,
        f64::MAX,
    ] {
        db.execute("insert into numbers values(?1)", [value])
            .unwrap();
    }
    // INTEGER storage is accepted by the original row.get::<f64>, including
    // the same rounding when i64 cannot be represented exactly as an f64.
    db.execute("insert into numbers values(?1)", [i64::MAX])
        .unwrap();
    let sql = "select value from numbers order by rowid";
    let original = db
        .prepare(sql)
        .unwrap()
        .query_map([], |row| row.get::<_, f64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let data = resource(sql, vec![text()]);
    assert_eq!(data.grant().columns, vec![text()]);
    let rows = read(&db, &data).unwrap();
    let expected = original
        .iter()
        .map(|value| vec![DataValue::Text(format!("{:016x}", value.to_bits()))])
        .collect::<Vec<_>>();
    assert_eq!(rows.rows, expected);
    assert_eq!(
        rows.rows[0],
        vec![DataValue::Text("3fefffffffffffff".into())]
    );
    assert_eq!(
        rows.rows[1],
        vec![DataValue::Text("0b777f245544ff83".into())]
    );
    let wire = serde_json::to_string(&rows).unwrap();
    let decoded: DataRows = serde_json::from_str(&wire).unwrap();
    assert_eq!(decoded, rows);
    for (row, value) in decoded.rows.iter().zip(original) {
        let DataValue::Text(bits) = &row[0] else {
            panic!("private float representation must be Text");
        };
        assert_eq!(bits.len(), 16);
        assert!(
            bits.bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert_eq!(u64::from_str_radix(bits, 16).unwrap(), value.to_bits());
    }
    assert!(db.is_autocommit());
}

#[test]
fn float_bits_do_not_add_finite_or_range_validation_to_row_get() {
    let db = Connection::open_in_memory().unwrap();
    for sql in ["select 1e999", "select -1e999", "select -0.0", "select -7"] {
        let original = db.query_row(sql, [], |row| row.get::<_, f64>(0)).unwrap();
        assert_eq!(
            read(&db, &resource(sql, vec![text()])).unwrap().rows,
            vec![vec![DataValue::Text(format!(
                "{:016x}",
                original.to_bits()
            ))]]
        );
    }
}

#[test]
fn float_bits_preserve_invalid_storage_and_first_column_first_row_failures() {
    let db = Connection::open_in_memory().unwrap();
    for sql in ["select null", "select '0.72'", "select x'01'"] {
        assert!(matches!(
            db.query_row(sql, [], |row| row.get::<_, f64>(0)),
            Err(rusqlite::Error::InvalidColumnType(..))
        ));
        let error = read(&db, &resource(sql, vec![text()])).unwrap_err();
        assert_eq!(error.code, "plugin_data_sql_failed");
        assert_eq!(error.phase, "read");
        assert_eq!(error.safe_cause, "invalid_column_type");
    }
    let data = resource(
        "select null,cast(x'80' as text) union all select abs(-9223372036854775808),'later private body'",
        vec![text(), text()],
    );
    let error = read_on_connection(
        &db,
        &data,
        &call(&data, &[]),
        RowPolicy::All,
        &[ColumnCodec::F64Bits, ColumnCodec::String],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "invalid_column_type");
    assert!(error.sqlite_code.is_none());
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("later private body")
    );
}

#[test]
fn float_bits_require_the_fixed_private_text_column_grant() {
    let db = Connection::open_in_memory().unwrap();
    for field in [
        DataField {
            data_type: DataType::Real,
            nullable: false,
        },
        DataField {
            nullable: true,
            ..text()
        },
    ] {
        let error = read(&db, &resource("select 0.72", vec![field])).unwrap_err();
        assert_eq!(error.code, "plugin_data_call_invalid");
        assert_eq!(error.safe_cause, "column_codec_mismatch");
    }
}
