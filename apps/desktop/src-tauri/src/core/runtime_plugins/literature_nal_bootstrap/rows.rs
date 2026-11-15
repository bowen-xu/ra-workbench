use super::error::{CaptureError, Result};
use crate::{
    data_interface::plugin_resources::{ColumnCodec, RowPolicy, read_on_connection},
    runtime_plugins::data::AcceptedDataResource,
};
use research_workbench_native_sdk::data::{DataCall, DataRows, DataValue};
use rusqlite::Connection;
use std::collections::VecDeque;

pub(super) fn call(resource: &AcceptedDataResource, arguments: Vec<DataValue>) -> DataCall {
    DataCall {
        id: resource.grant().id.clone(),
        version: resource.grant().version,
        arguments,
    }
}

pub(super) fn read(
    connection: &Connection,
    resource: &AcceptedDataResource,
    arguments: Vec<DataValue>,
    policy: RowPolicy,
    codecs: &[ColumnCodec],
) -> Result<DataRows> {
    read_on_connection(
        connection,
        resource,
        &call(resource, arguments),
        policy,
        codecs,
    )
    .map_err(CaptureError::Data)
}

/// Mechanical decoding after the original row.get primitive conversions.
pub(super) struct Row(VecDeque<DataValue>);

impl From<Vec<DataValue>> for Row {
    fn from(value: Vec<DataValue>) -> Self {
        Self(value.into())
    }
}

impl Row {
    pub(super) fn text(&mut self) -> Result<String> {
        match self.0.pop_front() {
            Some(DataValue::Text(value)) => Ok(value),
            _ => Err(CaptureError::Shape("text-column")),
        }
    }
    pub(super) fn optional_text(&mut self) -> Result<Option<String>> {
        match self.0.pop_front() {
            Some(DataValue::Text(value)) => Ok(Some(value)),
            Some(DataValue::Null) => Ok(None),
            _ => Err(CaptureError::Shape("optional-text-column")),
        }
    }
    pub(super) fn integer(&mut self) -> Result<i64> {
        match self.0.pop_front() {
            Some(DataValue::Integer(value)) => Ok(value),
            _ => Err(CaptureError::Shape("integer-column")),
        }
    }
    pub(super) fn boolean(&mut self) -> Result<bool> {
        self.integer().map(|value| value != 0)
    }
}
