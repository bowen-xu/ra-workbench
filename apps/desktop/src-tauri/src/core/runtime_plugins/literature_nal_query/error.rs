use crate::{
    data_interface::plugin_resources::PluginDataError, runtime_plugins::native::SafeDatabaseError,
};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum QueryOutcome {
    NotStarted,
    RolledBack,
    Committed {
        revision: i64,
    },
    Unknown {
        /// An observation inside an unsettled transaction, never a durable receipt.
        #[serde(skip_serializing_if = "Option::is_none")]
        observed_revision: Option<i64>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum QueryCause {
    Conflict,
    InvalidStructure,
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
}

impl fmt::Display for QueryCause {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Only QueryFailure's confirmed rollback path exposes the retry text.
            Self::Conflict => write!(out, "nal_query_cas_conflict"),
            Self::InvalidStructure => write!(out, "关系数据结构无效"),
            Self::Data(error) => write!(out, "{error}: {}", error.safe_cause),
            Self::Database(error) => error.fmt(out),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryFailure {
    pub(crate) cause: QueryCause,
    pub(crate) outcome: QueryOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl fmt::Display for QueryFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.cause, &self.outcome) {
            (QueryCause::Conflict, QueryOutcome::RolledBack) => {
                write!(out, "查询集合已变化，请重试")
            }
            (_, QueryOutcome::Unknown { .. }) => {
                write!(out, "查询集合提交或回滚结果未知，禁止自动重试。")
            }
            _ => self.cause.fmt(out),
        }?;
        if let Some(error) = &self.rollback_error {
            write!(out, " / rollback: {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for QueryFailure {}
