//! Original import-result content is distinct from a safe runtime diagnostic.
use std::fmt;

/// Only the existing import-result writer consumes this text. It is never a
/// diagnostic field: Serde skips the containing field and Debug is redacted.
/// Like raw_input, the persisted business record can contain source data.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct OriginalSqlFailure(String);

impl OriginalSqlFailure {
    pub(crate) fn capture(error: &rusqlite::Error) -> Self {
        Self(error.to_string())
    }

    pub(crate) fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OriginalSqlFailure {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str("OriginalSqlFailure([redacted])")
    }
}
