use crate::data_interface::plugin_resources::PluginDataError;
use serde::Serialize;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum SaveOutcome {
    NotStarted,
    RolledBack,
    Committed { item_id: String },
    Unknown,
}

pub(crate) use crate::runtime_plugins::native::SafeDatabaseError;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum SaveCause {
    Data(Box<PluginDataError>),
    Database(Box<SafeDatabaseError>),
}

impl fmt::Display for SaveCause {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(error) => write!(out, "{error}: {}", error.safe_cause),
            Self::Database(error) => error.fmt(out),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveFailure {
    pub(crate) cause: SaveCause,
    pub(crate) outcome: SaveOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rollback_error: Option<Box<SafeDatabaseError>>,
}

impl SaveFailure {
    pub(super) fn database(cause: SafeDatabaseError, outcome: SaveOutcome) -> Self {
        Self {
            cause: SaveCause::Database(Box::new(cause)),
            outcome,
            rollback_error: None,
        }
    }
}

impl fmt::Display for SaveFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}", self.cause)?;
        if let Some(error) = &self.rollback_error {
            write!(out, " / rollback: {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for SaveFailure {}
