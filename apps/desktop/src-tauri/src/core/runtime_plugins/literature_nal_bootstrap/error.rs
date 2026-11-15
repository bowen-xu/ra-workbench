use crate::{
    data_interface::plugin_resources::PluginDataError, runtime_plugins::native::SafeDatabaseError,
};
use serde::Serialize;
use std::fmt;

/// Safe private diagnostics; the original reachable caller maps public wording.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "diagnostic", rename_all = "camelCase")]
pub(crate) enum CaptureError {
    Data(Box<PluginDataError>),
    Owner(Box<SafeDatabaseError>),
    SnapshotChanged { cause: Option<Box<CaptureError>> },
    Shape(&'static str),
}

impl CaptureError {
    pub(super) fn changed(cause: Option<Self>) -> Self {
        Self::SnapshotChanged {
            cause: cause.map(Box::new),
        }
    }
    pub(super) fn same_failure(&self, other: &Self) -> bool {
        // These representations contain only safe structured diagnostic fields,
        // never SQL, bound values, provider responses or document content.
        match (serde_json::to_value(self), serde_json::to_value(other)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(error) => write!(out, "{error}: {}", error.safe_cause),
            Self::Owner(error) => error.fmt(out),
            Self::SnapshotChanged { cause } => {
                write!(out, "plugin_data_snapshot_changed")?;
                if let Some(cause) = cause {
                    write!(out, " / {cause}")?;
                }
                Ok(())
            }
            Self::Shape(reason) => write!(out, "nal_bootstrap_shape: {reason}"),
        }
    }
}

impl std::error::Error for CaptureError {}

pub(super) type Result<T> = std::result::Result<T, CaptureError>;
