//! Preserve the existing migration errors and structured metadata-write failures.
use crate::agent::catalog_commands::RenameCommandError;
use serde::Serialize;

#[derive(Debug)]
pub enum Error {
    InvalidInput,
    Unavailable,
    Conflict,
    Corrupt,
    Mutation(RenameCommandError),
}
impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::InvalidInput => serializer.serialize_str("invalidInput"),
            Self::Unavailable => serializer.serialize_str("unavailable"),
            Self::Conflict => serializer.serialize_str("conflict"),
            Self::Corrupt => serializer.serialize_str("corrupt"),
            Self::Mutation(error) => error.serialize(serializer),
        }
    }
}
