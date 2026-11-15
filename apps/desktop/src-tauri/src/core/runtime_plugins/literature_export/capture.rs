//! Original short guarded reads and mechanical SQLite value conversion.
use super::{ResearchExportError, ResearchExportManager, ResearchExportSql};
use research_workbench_native_sdk::research_export::{
    MAX_EXPORT_RECORDS, ResearchExportDatasetV1, ResearchExportSnapshotV1,
};
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Map, Number, Value};

pub(crate) fn capture(
    connection: &Connection,
    sql: &ResearchExportSql,
    manager: &ResearchExportManager,
) -> Result<ResearchExportSnapshotV1, ResearchExportError> {
    let exported_at =
        connection.query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get::<_, String>(0)
        })?;
    let mut datasets = Vec::with_capacity(sql.statements.len());
    let mut record_count = 0usize;
    for (name, statement) in &sql.statements {
        manager.check_cancelled()?;
        let records = query_dataset(connection, statement, manager)?;
        record_count = record_count.saturating_add(records.len());
        if record_count > MAX_EXPORT_RECORDS {
            return Err(ResearchExportError::TooLarge);
        }
        datasets.push(ResearchExportDatasetV1 {
            name: (*name).into(),
            records,
        });
    }
    Ok(ResearchExportSnapshotV1 {
        application_version: env!("CARGO_PKG_VERSION").into(),
        exported_at,
        datasets,
    })
}

fn query_dataset(
    connection: &Connection,
    sql: &str,
    manager: &ResearchExportManager,
) -> Result<Vec<Value>, ResearchExportError> {
    let mut statement = connection.prepare(sql)?;
    let columns = statement
        .column_names()
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    let rows = statement.query_map([], |row| {
        let mut record = Map::with_capacity(columns.len());
        for (index, column) in columns.iter().enumerate() {
            let is_json = column.ends_with("_json");
            let source_name = column.strip_suffix("_json").unwrap_or(column);
            record.insert(
                snake_to_camel(source_name),
                sqlite_value(row.get_ref(index)?, is_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        index,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
            );
        }
        Ok(Value::Object(record))
    })?;
    let mut records = Vec::new();
    for row in rows {
        manager.check_cancelled()?;
        records.push(row?);
        if records.len() > MAX_EXPORT_RECORDS {
            return Err(ResearchExportError::TooLarge);
        }
    }
    Ok(records)
}

fn sqlite_value(value: ValueRef<'_>, is_json: bool) -> Result<Value, serde_json::Error> {
    match value {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(value) => Ok(Value::Number(value.into())),
        ValueRef::Real(value) => Ok(Number::from_f64(value).map_or(Value::Null, Value::Number)),
        ValueRef::Text(bytes) => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            if is_json {
                serde_json::from_str(&text)
            } else {
                Ok(Value::String(text))
            }
        }
        ValueRef::Blob(_) => Err(serde_json::Error::io(std::io::Error::other(
            "blob columns are not exportable",
        ))),
    }
}

fn snake_to_camel(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut uppercase_next = false;
    for character in value.chars() {
        if character == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            output.extend(character.to_uppercase());
            uppercase_next = false;
        } else {
            output.push(character);
        }
    }
    output
}
