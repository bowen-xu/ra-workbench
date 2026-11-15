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

#[test]
fn fixed_column_order_preserves_the_original_first_conversion_failure() {
    let db = Connection::open_in_memory().unwrap();
    let data = resource("select 2147483648,null", vec![integer(true), text()]);
    let call = call(&data, &[]);
    let codecs = [ColumnCodec::OptionalI32, ColumnCodec::String];
    let natural = read_on_connection(&db, &data, &call, RowPolicy::All, &codecs).unwrap_err();
    assert_eq!(natural.code, "plugin_data_sql_failed");
    assert_eq!(natural.phase, "read");
    assert_eq!(natural.safe_cause, "integral_value_out_of_range");

    let ordered = read_on_connection_in_order(&db, &data, &call, RowPolicy::All, &codecs, &[1, 0])
        .unwrap_err();
    assert_eq!(ordered.code, "plugin_data_sql_failed");
    assert_eq!(ordered.phase, "read");
    assert_eq!(ordered.safe_cause, "invalid_column_type");
    assert!(ordered.sqlite_code.is_none() && ordered.sqlite_extended_code.is_none());
    assert!(db.is_autocommit());
}

#[test]
fn ordered_conversion_keeps_sql_column_positions_and_original_row_policies() {
    let db = Connection::open_in_memory().unwrap();
    let data = resource(
        "select 'first',7,'last' union all select cast(x'80' as text),8,'later'",
        vec![text(), integer(false), text()],
    );
    let codecs = [ColumnCodec::String, ColumnCodec::I64, ColumnCodec::String];
    for policy in [RowPolicy::FirstOptional, RowPolicy::FirstRequired] {
        let rows =
            read_on_connection_in_order(&db, &data, &call(&data, &[]), policy, &codecs, &[2, 0, 1])
                .unwrap();
        assert_eq!(
            rows.rows,
            vec![vec![
                DataValue::Text("first".into()),
                DataValue::Integer(7),
                DataValue::Text("last".into()),
            ]]
        );
    }
    let error = read_on_connection_in_order(
        &db,
        &data,
        &call(&data, &[]),
        RowPolicy::All,
        &codecs,
        &[2, 0, 1],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "column_conversion_failed");

    let absent = resource(
        "select 'empty',0,'empty' where 0",
        vec![text(), integer(false), text()],
    );
    let error = read_on_connection_in_order(
        &db,
        &absent,
        &call(&absent, &[]),
        RowPolicy::FirstRequired,
        &codecs,
        &[2, 0, 1],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "query_returned_no_rows");
    assert!(db.is_autocommit());
}

#[test]
fn invalid_column_orders_are_rejected_before_stepping_the_statement() {
    let db = Connection::open_in_memory().unwrap();
    let data = resource(
        "select abs(-9223372036854775808),7",
        vec![integer(false), integer(false)],
    );
    let codecs = [ColumnCodec::I64, ColumnCodec::I64];
    for order in [
        &[][..],
        &[0][..],
        &[0, 0][..],
        &[0, 2][..],
        &[usize::MAX, 0][..],
        &[0, 1, 2][..],
    ] {
        let error = read_on_connection_in_order(
            &db,
            &data,
            &call(&data, &[]),
            RowPolicy::All,
            &codecs,
            order,
        )
        .unwrap_err();
        assert_eq!(error.code, "plugin_data_call_invalid");
        assert_eq!(error.phase, "validate");
        assert_eq!(error.safe_cause, "column_order_mismatch");
        assert!(error.sqlite_code.is_none() && error.sqlite_extended_code.is_none());
    }
}

#[test]
fn ordered_reader_preserves_resource_authorization_and_codec_contracts() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table items(value text); insert into items values('retained');")
        .unwrap();
    let package = PackageFixture::new(&[("write.sql", "delete from items")]);
    let mutation = package.resource("write.sql", DataAccess::Write, vec![], vec![]);
    let error = read_on_connection_in_order(
        &db,
        &mutation,
        &call(&mutation, &[]),
        RowPolicy::All,
        &[],
        &[],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "resource_access_mismatch");
    assert_eq!(
        db.query_row("select count(*) from items", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let data = resource("select value from items", vec![text()]);
    let error = read_on_connection_in_order(
        &db,
        &data,
        &call(&data, &[]),
        RowPolicy::All,
        &[ColumnCodec::I64],
        &[],
    )
    .unwrap_err();
    assert_eq!(error.safe_cause, "column_codec_mismatch");
    assert!(db.is_autocommit());
}
