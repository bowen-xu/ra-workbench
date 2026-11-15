#[cfg(feature = "development-data")]
use std::path::{Component, Path, PathBuf};

use crate::storage::StorageError;

const PRODUCTION_IDENTIFIER: &str = "com.bowenxu.research-assistant";
const DEVELOPMENT_IDENTIFIER: &str = "com.bowenxu.research-assistant.dev";

#[cfg(feature = "development-data")]
const DEVELOPMENT_DATA_DIRECTORY_VARIABLE: &str = "RESEARCH_ASSISTANT_DEVELOPMENT_DATA_DIRECTORY";

#[cfg(feature = "development-data")]
pub(crate) fn development_data_directory_override() -> Result<Option<PathBuf>, StorageError> {
    let Some(value) = std::env::var_os(DEVELOPMENT_DATA_DIRECTORY_VARIABLE) else {
        return Ok(None);
    };
    validate_development_data_directory(Path::new(&value)).map(Some)
}

#[cfg(feature = "development-data")]
fn validate_development_data_directory(path: &Path) -> Result<PathBuf, StorageError> {
    let is_safe_shape = path.is_absolute()
        && path.parent().is_some_and(|parent| parent != Path::new("/"))
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir));
    if !is_safe_shape {
        return Err(StorageError::UnsafeDataEnvironment(format!(
            "{DEVELOPMENT_DATA_DIRECTORY_VARIABLE} 必须是非根目录下、不含 `..` 的绝对路径"
        )));
    }
    Ok(path.to_path_buf())
}

pub(crate) fn validate_data_environment(
    actual_identifier: &str,
    is_debug_build: bool,
    uses_development_data: bool,
) -> Result<(), StorageError> {
    if is_debug_build && !uses_development_data {
        return Err(StorageError::UnsafeDataEnvironment(
            "debug 构建必须启用 development-data 特性".to_owned(),
        ));
    }

    let expected_identifier = if uses_development_data {
        DEVELOPMENT_IDENTIFIER
    } else {
        PRODUCTION_IDENTIFIER
    };
    if actual_identifier != expected_identifier {
        return Err(StorageError::UnsafeDataEnvironment(format!(
            "当前构建要求 identifier `{expected_identifier}`，实际为 `{actual_identifier}`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_matching_identifiers() {
        validate_data_environment(DEVELOPMENT_IDENTIFIER, true, true)
            .expect("debug development build");
        validate_data_environment(DEVELOPMENT_IDENTIFIER, false, true)
            .expect("release development build");
        validate_data_environment(PRODUCTION_IDENTIFIER, false, false)
            .expect("release production build");

        assert!(validate_data_environment(PRODUCTION_IDENTIFIER, true, false).is_err());
        assert!(validate_data_environment(PRODUCTION_IDENTIFIER, true, true).is_err());
        assert!(validate_data_environment(DEVELOPMENT_IDENTIFIER, false, false).is_err());
    }

    #[cfg(feature = "development-data")]
    #[test]
    fn development_data_override_requires_a_scoped_absolute_path() {
        assert_eq!(
            validate_development_data_directory(Path::new("/tmp/research-assistant-fixture"))
                .unwrap(),
            PathBuf::from("/tmp/research-assistant-fixture")
        );
        assert!(validate_development_data_directory(Path::new("relative")).is_err());
        assert!(validate_development_data_directory(Path::new("/tmp/../private-data")).is_err());
        assert!(validate_development_data_directory(Path::new("/tmp")).is_err());
        assert!(validate_development_data_directory(Path::new("/")).is_err());
    }
}
