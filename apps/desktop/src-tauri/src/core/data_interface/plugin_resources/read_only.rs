//! Existing row.get conversion contracts selected only by fixed Core metadata.
use super::*;
use research_workbench_native_sdk::data::{DataField, DataType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowPolicy {
    All,
    FirstOptional,
    FirstRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColumnCodec {
    String,
    OptionalString,
    OptionalI32,
    OptionalI64,
    I32,
    I64,
    U64,
    Usize,
    Bool,
    F64Bits,
    OptionalF32Bits,
}

impl ColumnCodec {
    pub(crate) fn field(self) -> DataField {
        DataField {
            data_type: match self {
                Self::String | Self::OptionalString | Self::F64Bits | Self::OptionalF32Bits => {
                    DataType::Text
                }
                _ => DataType::Integer,
            },
            nullable: matches!(
                self,
                Self::OptionalString
                    | Self::OptionalI32
                    | Self::OptionalI64
                    | Self::OptionalF32Bits
            ),
        }
    }

    fn decode(self, row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<DataValue> {
        Ok(match self {
            Self::String => DataValue::Text(row.get::<_, String>(index)?),
            Self::OptionalString => row
                .get::<_, Option<String>>(index)?
                .map_or(DataValue::Null, DataValue::Text),
            Self::OptionalI32 => row
                .get::<_, Option<i32>>(index)?
                .map_or(DataValue::Null, |value| {
                    DataValue::Integer(i64::from(value))
                }),
            Self::OptionalI64 => row
                .get::<_, Option<i64>>(index)?
                .map_or(DataValue::Null, DataValue::Integer),
            Self::I32 => DataValue::Integer(i64::from(row.get::<_, i32>(index)?)),
            Self::I64 => DataValue::Integer(row.get::<_, i64>(index)?),
            // SQLite integers are i64. rusqlite rejects negatives before this
            // normalization, so a successful u64 is still inside i64::MAX.
            Self::U64 => DataValue::Integer(row.get::<_, u64>(index)? as i64),
            Self::Usize => DataValue::Integer(row.get::<_, usize>(index)? as i64),
            Self::Bool => DataValue::Integer(i64::from(row.get::<_, bool>(index)?)),
            // Keep the original row.get conversion, including integer-to-f64
            // and non-finite REAL values. JSON numbers cannot retain every bit.
            Self::F64Bits => {
                DataValue::Text(format!("{:016x}", row.get::<_, f64>(index)?.to_bits()))
            }
            Self::OptionalF32Bits => row
                .get::<_, Option<f32>>(index)?
                .map_or(DataValue::Null, |value| {
                    DataValue::Text(format!("{:08x}", value.to_bits()))
                }),
        })
    }
}

/// A fixed Core decoder may interpret each row before the cursor advances.
/// Mapping errors retain their original owner, separately from SQL diagnostics.
pub(crate) enum MappedReadError<E> {
    Data(Box<PluginDataError>),
    Mapping(E),
}
impl<E> From<Box<PluginDataError>> for MappedReadError<E> {
    fn from(error: Box<PluginDataError>) -> Self {
        Self::Data(error)
    }
}
pub(crate) fn read_on_connection_mapped<T, E>(
    connection: &Connection,
    resource: &AcceptedDataResource,
    call: &DataCall,
    policy: RowPolicy,
    codecs: &[ColumnCodec],
    mut map: impl FnMut(Vec<DataValue>) -> std::result::Result<T, E>,
) -> std::result::Result<Vec<T>, MappedReadError<E>> {
    let mut statement = prepare_statement(
        connection,
        resource,
        call,
        DataAccess::Read,
        TransactionRequirement::NotRequired,
    )?;
    validate_codecs(resource, codecs)?;
    query_rows_mapped(
        &mut statement,
        resource,
        call,
        policy,
        None,
        |row, index| {
            codecs[index]
                .decode(row, index)
                .map_err(|error| sql_failure(resource, "read", &error))
        },
        |fields| {
            validate_result(
                resource,
                &DataResult::Rows(DataRows {
                    rows: vec![fields.clone()],
                }),
            )?;
            map(fields).map_err(MappedReadError::Mapping)
        },
    )
}

/// Borrow the caller's original guard, including its uncommitted values. No
/// transaction is started or required, and this result is not a recheck proof.
/// Neither the row policy nor codecs can be selected by a native request.
pub(crate) fn read_on_connection(
    connection: &Connection,
    resource: &AcceptedDataResource,
    call: &DataCall,
    policy: RowPolicy,
    codecs: &[ColumnCodec],
) -> Result<DataRows> {
    read(connection, resource, call, policy, codecs, None)
}

/// Preserve an existing row.get order selected by a fixed Core caller. The
/// complete permutation changes conversion precedence, never SQL column slots.
/// It is not selectable by a data call or native request.
pub(crate) fn read_on_connection_in_order(
    connection: &Connection,
    resource: &AcceptedDataResource,
    call: &DataCall,
    policy: RowPolicy,
    codecs: &[ColumnCodec],
    column_order: &[usize],
) -> Result<DataRows> {
    read(
        connection,
        resource,
        call,
        policy,
        codecs,
        Some(column_order),
    )
}

fn read(
    connection: &Connection,
    resource: &AcceptedDataResource,
    call: &DataCall,
    policy: RowPolicy,
    codecs: &[ColumnCodec],
    column_order: Option<&[usize]>,
) -> Result<DataRows> {
    let mut statement = prepare_statement(
        connection,
        resource,
        call,
        DataAccess::Read,
        TransactionRequirement::NotRequired,
    )?;
    validate_codecs(resource, codecs)?;
    if let Some(order) = column_order {
        validate_column_order(resource, codecs.len(), order)?;
    }
    let rows = query_rows(
        &mut statement,
        resource,
        call,
        policy,
        column_order,
        |row, index| {
            codecs[index]
                .decode(row, index)
                .map_err(|error| sql_failure(resource, "read", &error))
        },
    )?;
    let result = DataResult::Rows(rows);
    validate_result(resource, &result)?;
    match result {
        DataResult::Rows(rows) => Ok(rows),
        DataResult::Affected(_) => unreachable!("read-only adapter only returns rows"),
    }
}

fn validate_codecs(resource: &AcceptedDataResource, codecs: &[ColumnCodec]) -> Result<()> {
    if codecs.len() != resource.grant().columns.len()
        || codecs
            .iter()
            .zip(&resource.grant().columns)
            .any(|(codec, field)| codec.field() != *field)
    {
        let mut error = failure(
            resource,
            "validate",
            "plugin_data_call_invalid",
            "数据操作的 Core 列转换与已授予契约不符。",
        );
        error.safe_cause = "column_codec_mismatch".into();
        return Err(error);
    }
    Ok(())
}

fn validate_column_order(
    resource: &AcceptedDataResource,
    column_count: usize,
    order: &[usize],
) -> Result<()> {
    let invalid = || {
        let mut error = failure(
            resource,
            "validate",
            "plugin_data_call_invalid",
            "数据操作的 Core 列转换顺序不是完整列排列。",
        );
        error.safe_cause = "column_order_mismatch".into();
        error
    };
    if order.len() != column_count {
        return Err(invalid());
    }
    let mut seen = vec![false; column_count];
    for &index in order {
        if index >= column_count || seen[index] {
            return Err(invalid());
        }
        seen[index] = true;
    }
    Ok(())
}
