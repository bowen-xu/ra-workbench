//! Original source-relative path policy shared by existing native file adapters.
use super::TextFileError;
use std::path::PathBuf;

pub(crate) fn validate_relative_path(value: &str) -> Result<PathBuf, TextFileError> {
    research_workbench_native_sdk::resource_path::validate_relative_path(value)
        .map_err(|_| TextFileError::InvalidPath)
}
