//! Fixed, package-owned statements borrowed from existing Core connections.
//!
//! This adapter neither starts nor commits a transaction. A typed Core caller
//! retains resource authorization, shared writes, receipts and unknown outcomes.
//! No database guard survives in the owned read passed to a native calculation.
use crate::runtime_plugins::data::AcceptedDataResource;
use research_workbench_native_sdk::data::{DataAccess, DataCall, DataResult, DataRows, DataValue};
use rusqlite::{
    Connection, Statement, Transaction, params_from_iter, types::Value, types::ValueRef,
};
use serde::Serialize;

mod read_only;
pub(crate) use read_only::{
    ColumnCodec, MappedReadError, RowPolicy, read_on_connection, read_on_connection_in_order,
    read_on_connection_mapped,
};
mod prepared_write;
pub(crate) use prepared_write::PreparedDataWrite;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransactionRequirement {
    Required,
    NotRequired,
}

/// Safe diagnostics omit SQL and values. Original import-result text is private
/// business content, skipped by Serde and redacted by its own Debug adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code}: {message}")]
pub(crate) struct PluginDataError {
    pub code: &'static str,
    pub message: &'static str,
    pub package_key: String,
    pub resource_id: String,
    pub resource_version: u32,
    pub phase: &'static str,
    pub safe_cause: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqlite_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sqlite_extended_code: Option<i32>,
    #[serde(skip)]
    pub(crate) original_sql_failure: Option<super::original_sql_failure::OriginalSqlFailure>,
}

type Result<T> = std::result::Result<T, Box<PluginDataError>>;

/// One complete read, including predicate/absence results, not a log watermark.
/// The caller captures all of its dependencies in the same original read tx.
pub(crate) struct PreparedDataRead {
    resource: AcceptedDataResource,
    call: DataCall,
    rows: DataRows,
}

impl PreparedDataRead {
    pub(crate) fn capture(
        tx: &Transaction<'_>,
        resource: AcceptedDataResource,
        call: DataCall,
    ) -> Result<Self> {
        let rows = read_in(tx, &resource, &call)?;
        Ok(Self {
            resource,
            call,
            rows,
        })
    }

    pub(crate) fn rows(&self) -> &DataRows {
        &self.rows
    }

    /// Run this before the caller's dependent writes, inside its final tx.
    /// The original bound parameters and accepted resource cannot be replaced
    /// by a child-supplied claim about which values remained unchanged.
    pub(crate) fn recheck_in<'tx, 'connection>(
        &'tx self,
        tx: &'tx Transaction<'connection>,
    ) -> Result<CheckedDataRead<'tx, 'connection>> {
        if read_in(tx, &self.resource, &self.call)? != self.rows {
            return Err(failure(
                &self.resource,
                "recheck",
                "plugin_data_snapshot_changed",
                "插件计算所依据的数据已变化；本次尚未提交。",
            ));
        }
        Ok(CheckedDataRead {
            resource: &self.resource,
            tx,
        })
    }
}

/// Borrowed proof for this transaction, never a serializable transaction handle.
/// Additional reads and physical/Core permissions remain the typed caller's job.
pub(crate) struct CheckedDataRead<'tx, 'connection> {
    resource: &'tx AcceptedDataResource,
    tx: &'tx Transaction<'connection>,
}

impl CheckedDataRead<'_, '_> {
    pub(crate) fn execute(&self, mutation: &AcceptedDataResource, call: &DataCall) -> Result<u64> {
        if mutation.owner_id() != self.resource.owner_id()
            || mutation.package_key() != self.resource.package_key()
        {
            return Err(failure(
                mutation,
                "authorize",
                "plugin_data_binding_mismatch",
                "数据写入不属于本次已核对的插件包。",
            ));
        }
        write_in(self.tx, mutation, call)
    }
}

/// Execute one fixed write authorized by the typed Core caller in its original
/// transaction. This does not establish a read-dependency proof or authorize a
/// child-supplied resource; callers retain package, scope and read-set checks.
pub(crate) fn write_in(
    tx: &Transaction<'_>,
    resource: &AcceptedDataResource,
    call: &DataCall,
) -> Result<u64> {
    match execute_on_connection(
        tx,
        resource,
        call,
        DataAccess::Write,
        TransactionRequirement::Required,
    )? {
        DataResult::Affected(count) => Ok(count),
        DataResult::Rows(_) => unreachable!("write adapter only returns affected rows"),
    }
}

/// Execute one authorized statement without changing the original connection's
/// transaction boundary. SQL errors and zero affected rows do not prove that a
/// trigger made no changes; the typed caller retains that outcome distinction.
pub(crate) fn write_on_connection(
    connection: &Connection,
    resource: &AcceptedDataResource,
    call: &DataCall,
) -> Result<u64> {
    match execute_on_connection(
        connection,
        resource,
        call,
        DataAccess::Write,
        TransactionRequirement::NotRequired,
    )? {
        DataResult::Affected(count) => Ok(count),
        DataResult::Rows(_) => unreachable!("write adapter only returns affected rows"),
    }
}

fn read_in(
    tx: &Transaction<'_>,
    resource: &AcceptedDataResource,
    call: &DataCall,
) -> Result<DataRows> {
    match execute_on_connection(
        tx,
        resource,
        call,
        DataAccess::Read,
        TransactionRequirement::Required,
    )? {
        DataResult::Rows(rows) => Ok(rows),
        DataResult::Affected(_) => unreachable!("read adapter only returns rows"),
    }
}

fn execute_on_connection(
    connection: &Connection,
    resource: &AcceptedDataResource,
    call: &DataCall,
    access: DataAccess,
    transaction: TransactionRequirement,
) -> Result<DataResult> {
    let mut statement = prepare_statement(connection, resource, call, access, transaction)?;
    let result = match access {
        DataAccess::Write => execute_write(&mut statement, resource, call)?,
        DataAccess::Read => DataResult::Rows(query_rows(
            &mut statement,
            resource,
            call,
            RowPolicy::All,
            None,
            |row, index| {
                let value = row
                    .get_ref(index)
                    .map_err(|error| sql_failure(resource, "read", &error))?;
                data_value(value).ok_or_else(|| {
                    failure(
                        resource,
                        "decode",
                        "plugin_data_result_invalid",
                        "插件数据结果不是有效的 UTF-8 或有限数值。",
                    )
                })
            },
        )?),
    };
    validate_result(resource, &result)?;
    Ok(result)
}

fn execute_write(
    statement: &mut Statement<'_>,
    resource: &AcceptedDataResource,
    call: &DataCall,
) -> Result<DataResult> {
    let values = call.arguments.iter().map(sql_value).collect::<Vec<_>>();
    let count = statement
        .execute(params_from_iter(values.iter()))
        .map_err(|error| sql_failure(resource, "execute", &error))?;
    Ok(DataResult::Affected(count as u64))
}

fn prepare_statement<'connection>(
    connection: &'connection Connection,
    resource: &AcceptedDataResource,
    call: &DataCall,
    access: DataAccess,
    transaction: TransactionRequirement,
) -> Result<Statement<'connection>> {
    let invalid = || {
        failure(
            resource,
            "validate",
            "plugin_data_call_invalid",
            "数据操作的名称、版本、参数或读写种类与已授予契约不符。",
        )
    };
    if resource.grant().access != access {
        let mut error = invalid();
        error.safe_cause = "resource_access_mismatch".into();
        return Err(error);
    }
    if transaction == TransactionRequirement::Required && connection.is_autocommit() {
        let mut error = invalid();
        error.safe_cause = "caller_transaction_missing".into();
        return Err(error);
    }
    call.validate_for(resource.grant()).map_err(|cause| {
        let mut error = invalid();
        // The SDK diagnostics contain only shape/indices/types, never values.
        error.safe_cause = cause.to_string();
        error
    })?;
    // Do not use the connection's statement cache: the accepted bytes and their
    // lifetime are explicit, and all statements/rows drop before returning.
    let statement = connection
        .prepare(resource.sql())
        .map_err(|error| sql_failure(resource, "prepare", &error))?;
    if statement.parameter_count() != resource.grant().parameters.len()
        || statement.column_count() != resource.grant().columns.len()
        || statement.readonly() != (access == DataAccess::Read)
    {
        return Err(failure(
            resource,
            "prepare",
            "plugin_data_statement_incompatible",
            "插件数据语句与已授予参数、列或读写种类不兼容。",
        ));
    }
    Ok(statement)
}

fn query_rows(
    statement: &mut Statement<'_>,
    resource: &AcceptedDataResource,
    call: &DataCall,
    policy: RowPolicy,
    column_order: Option<&[usize]>,
    decode: impl FnMut(&rusqlite::Row<'_>, usize) -> Result<DataValue>,
) -> Result<DataRows> {
    query_rows_mapped(statement, resource, call, policy, column_order, decode, Ok)
        .map(|rows| DataRows { rows })
}

fn query_rows_mapped<T, E: From<Box<PluginDataError>>>(
    statement: &mut Statement<'_>,
    resource: &AcceptedDataResource,
    call: &DataCall,
    policy: RowPolicy,
    column_order: Option<&[usize]>,
    mut decode: impl FnMut(&rusqlite::Row<'_>, usize) -> Result<DataValue>,
    mut map: impl FnMut(Vec<DataValue>) -> std::result::Result<T, E>,
) -> std::result::Result<Vec<T>, E> {
    let values = call.arguments.iter().map(sql_value).collect::<Vec<_>>();
    let mut cursor = statement
        .query(params_from_iter(values.iter()))
        .map_err(|error| sql_failure(resource, "query", &error))?;
    let mut rows = Vec::new();
    while let Some(row) = cursor
        .next()
        .map_err(|error| sql_failure(resource, "read", &error))?
    {
        let fields = match column_order {
            Some(order) => {
                // The fixed Core reader validates a complete permutation. Keep
                // every decoded value in its original SQL result column.
                let mut fields = vec![DataValue::Null; resource.grant().columns.len()];
                for &index in order {
                    fields[index] = decode(row, index)?;
                }
                fields
            }
            None => {
                let mut fields = Vec::with_capacity(resource.grant().columns.len());
                for index in 0..resource.grant().columns.len() {
                    fields.push(decode(row, index)?);
                }
                fields
            }
        };
        rows.push(map(fields)?);
        if matches!(policy, RowPolicy::FirstOptional | RowPolicy::FirstRequired) {
            break;
        }
    }
    if rows.is_empty() && policy == RowPolicy::FirstRequired {
        return Err(sql_failure(resource, "read", &rusqlite::Error::QueryReturnedNoRows).into());
    }
    Ok(rows)
}

fn validate_result(resource: &AcceptedDataResource, result: &DataResult) -> Result<()> {
    result.validate_for(resource.grant()).map_err(|cause| {
        let mut error = failure(
            resource,
            "decode",
            "plugin_data_result_invalid",
            "插件数据结果与已授予列类型不符。",
        );
        error.safe_cause = cause.to_string();
        error
    })
}

fn sql_value(value: &DataValue) -> Value {
    match value {
        DataValue::Null => Value::Null,
        DataValue::Integer(value) => Value::Integer(*value),
        DataValue::Real(value) => Value::Real(*value),
        DataValue::Text(value) => Value::Text(value.clone()),
        DataValue::Blob(value) => Value::Blob(value.clone()),
    }
}

fn data_value(value: ValueRef<'_>) -> Option<DataValue> {
    Some(match value {
        ValueRef::Null => DataValue::Null,
        ValueRef::Integer(value) => DataValue::Integer(value),
        ValueRef::Real(value) if value.is_finite() => DataValue::Real(value),
        ValueRef::Real(_) => return None,
        ValueRef::Text(value) => DataValue::Text(std::str::from_utf8(value).ok()?.to_owned()),
        ValueRef::Blob(value) => DataValue::Blob(value.to_vec()),
    })
}

fn failure(
    resource: &AcceptedDataResource,
    phase: &'static str,
    code: &'static str,
    message: &'static str,
) -> Box<PluginDataError> {
    Box::new(PluginDataError {
        code,
        message,
        package_key: resource.package_key().to_owned(),
        resource_id: resource.grant().id.clone(),
        resource_version: resource.grant().version,
        phase,
        safe_cause: code.into(),
        sqlite_code: None,
        sqlite_extended_code: None,
        original_sql_failure: None,
    })
}

pub(crate) fn sql_failure(
    resource: &AcceptedDataResource,
    phase: &'static str,
    error: &rusqlite::Error,
) -> Box<PluginDataError> {
    let mut result = failure(
        resource,
        phase,
        "plugin_data_sql_failed",
        "插件数据操作失败；当前事务由原 Core 调用方处理。",
    );
    result.original_sql_failure = Some(super::original_sql_failure::OriginalSqlFailure::capture(
        error,
    ));
    let code = match error {
        rusqlite::Error::SqliteFailure(code, _)
        | rusqlite::Error::SqlInputError { error: code, .. } => Some(code),
        _ => None,
    };
    if let Some(code) = code {
        result.sqlite_code = Some(format!("{:?}", code.code));
        result.sqlite_extended_code = Some(code.extended_code);
    }
    result.safe_cause = match error {
        rusqlite::Error::SqliteFailure(code, _)
        | rusqlite::Error::SqlInputError { error: code, .. } => format!("{:?}", code.code),
        rusqlite::Error::MultipleStatement => "multiple_statements".into(),
        rusqlite::Error::InvalidQuery => "invalid_query".into(),
        rusqlite::Error::InvalidParameterCount(_, _) => "parameter_count_mismatch".into(),
        rusqlite::Error::InvalidParameterName(_) => "invalid_parameter_name".into(),
        rusqlite::Error::InvalidColumnType(_, _, _) => "invalid_column_type".into(),
        rusqlite::Error::InvalidColumnIndex(_) => "invalid_column_index".into(),
        rusqlite::Error::IntegralValueOutOfRange(_, _) => "integral_value_out_of_range".into(),
        rusqlite::Error::FromSqlConversionFailure(_, _, _) => "column_conversion_failed".into(),
        rusqlite::Error::Utf8Error(_) => "invalid_utf8".into(),
        rusqlite::Error::QueryReturnedNoRows => "query_returned_no_rows".into(),
        _ => "statement_failed".into(),
    };
    result
}

#[cfg(test)]
mod tests;
