use serde::Serialize;
use std::fmt;

/// SQLite messages may contain SQL, parameter values or a trigger's private text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafeDatabaseError {
    pub(crate) code: &'static str,
    pub(crate) phase: &'static str,
    pub(crate) safe_cause: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sqlite_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sqlite_extended_code: Option<i32>,
    #[serde(skip)]
    pub(crate) original_sql_failure:
        Option<crate::data_interface::original_sql_failure::OriginalSqlFailure>,
}

impl SafeDatabaseError {
    pub(crate) fn sql(phase: &'static str, cause: &rusqlite::Error) -> Self {
        let sqlite = match cause {
            rusqlite::Error::SqliteFailure(error, _)
            | rusqlite::Error::SqlInputError { error, .. } => Some(error),
            _ => None,
        };
        Self {
            code: "storage_error",
            phase,
            safe_cause: "sqlite_operation_failed",
            sqlite_code: sqlite.map(|error| format!("{:?}", error.code)),
            sqlite_extended_code: sqlite.map(|error| error.extended_code),
            original_sql_failure: Some(
                crate::data_interface::original_sql_failure::OriginalSqlFailure::capture(cause),
            ),
        }
    }

    pub(crate) fn invariant(code: &'static str, phase: &'static str) -> Self {
        Self {
            code,
            phase,
            safe_cause: code,
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        }
    }
}

impl fmt::Display for SafeDatabaseError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}: {} ({})", self.code, self.safe_cause, self.phase)?;
        if let Some(code) = &self.sqlite_code {
            write!(out, " / {code}")?;
        }
        if let Some(code) = self.sqlite_extended_code {
            write!(out, " / {code}")?;
        }
        Ok(())
    }
}
