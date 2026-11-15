//! The deployed journal operation UUID is UUIDv5(conversation, batch bytes).
//! This pure function also indexes legacy batches that predate log receipts.
use rusqlite::{Connection, functions::FunctionFlags, types::ValueRef};
use uuid::Uuid;

pub(crate) fn register(connection: &Connection) -> rusqlite::Result<()> {
    // The callback depends only on two UUID inputs, never on storage or process
    // state. INNOCUOUS permits its CHECK/trigger use with trusted_schema=OFF;
    // DIRECTONLY would prohibit precisely those schema uses.
    connection.create_scalar_function(
        "data_journal_operation_id",
        2,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let conversation = input_uuid(context.get_raw(0))?;
            let batch = input_uuid(context.get_raw(1))?;
            Ok(Uuid::new_v5(&conversation, batch.as_bytes()).to_string())
        },
    )
}

fn input_uuid(value: ValueRef<'_>) -> rusqlite::Result<Uuid> {
    let parsed = match value {
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| Uuid::parse_str(text).ok())
            .filter(|id| !id.is_nil()),
        _ => None,
    };
    parsed.ok_or_else(|| {
        rusqlite::Error::UserFunctionError(
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "DATA_JOURNAL_IDENTITY_INVALID: expected non-nil UUID text",
            )
            .into(),
        )
    })
}

/// Old-schema startup replay runs before 211 creates SQL-only receipts. Once
/// present, reject their UUIDs before a legacy journal retry or new log accept.
pub(super) fn sql_receipt_exists(
    connection: &Connection,
    operation: Uuid,
) -> rusqlite::Result<bool> {
    let deployed: bool = connection.query_row(
        "select exists(select 1 from sqlite_schema where type='table' and name='data_sql_operation_receipts')",
        [],
        |row| row.get(0),
    )?;
    if !deployed {
        return Ok(false);
    }
    connection.query_row(
        "select exists(select 1 from data_sql_operation_receipts where operation_id=?1)",
        [operation.to_string()],
        |row| row.get(0),
    )
}

#[cfg(test)]
#[path = "operation_identity_tests.rs"]
mod tests;
