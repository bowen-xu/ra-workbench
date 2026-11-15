//! Required literature metadata source over the caller's current SQL snapshot.
//! This internal repository port does not grant plugins general database access.
use rusqlite::Connection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaperMetadataError {
    SourceUnavailable,
    Unavailable,
}

pub(crate) type PaperMetadataSource = fn(&Connection, &str) -> Result<String, PaperMetadataError>;
