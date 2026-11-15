//! Required portrait body composition on the caller's existing connection.
//! The function value neither owns a database nor grants arbitrary SQL access.
use rusqlite::Connection;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaperPortraitError {
    #[expect(
        dead_code,
        reason = "The Core portrait callback retains the transient-failure classification."
    )]
    Unavailable,
    #[expect(
        dead_code,
        reason = "The Core portrait callback retains the missing-source classification."
    )]
    SourceUnavailable,
}
pub(crate) type PaperPortraitSource = fn(&Connection, &str) -> Result<String, PaperPortraitError>;
