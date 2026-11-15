//! The original native text-file mechanism behind a manuscript-bound capability.
//! Construction does not validate or access disk: each operation retains its
//! original UUID, path, size, symlink and compare-and-swap check order.
mod atomic;
mod directories;
pub(crate) mod legacy;
mod path;

use std::{fs, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum TextFileError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("稿件标识无效")]
    InvalidManuscriptId,
    #[error("稿件路径无效")]
    InvalidPath,
    #[error("稿件文件不存在")]
    FileNotFound,
    #[error("文件已在其他位置被修改，请重新载入后再保存")]
    FileConflict,
    #[error("文本文件超过 8 MiB 限制")]
    FileTooLarge,
    #[error("第一版只支持 UTF-8 文本文件")]
    UnsupportedEncoding,
    #[error("稿件工程不允许符号链接")]
    SymlinkNotAllowed,
}

impl TextFileError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "storage_error",
            Self::InvalidManuscriptId | Self::InvalidPath => "invalid_input",
            Self::FileNotFound => "not_found",
            Self::FileConflict => "file_conflict",
            Self::FileTooLarge => "file_too_large",
            Self::UnsupportedEncoding => "unsupported_encoding",
            Self::SymlinkNotAllowed => "unsafe_path",
        }
    }
}

/// Owned by the same application runtime; no root or absolute-path getter.
pub(crate) struct ScopedTextFiles {
    directory: PathBuf,
    manuscript_id: String,
}

/// Owned staging identity remains inside Core, outside all Native snapshots.
pub(crate) struct StagedDirectory {
    path: PathBuf,
}
impl StagedDirectory {
    pub(crate) fn identity(&self) -> String {
        self.path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
    pub(crate) fn discard(self) -> Result<(), TextFileError> {
        fs::remove_dir_all(self.path).map_err(Into::into)
    }
}

impl ScopedTextFiles {
    pub(crate) fn directories(&self) -> Result<Vec<String>, TextFileError> {
        legacy::list_directories(&self.directory, &self.manuscript_id)
    }
    pub(crate) fn create_directory(&self, path: &str) -> Result<(), TextFileError> {
        legacy::create_directory(&self.directory, &self.manuscript_id, path)
    }
    pub(crate) fn rename_directory(&self, from: &str, to: &str) -> Result<(), TextFileError> {
        legacy::rename_directory(&self.directory, &self.manuscript_id, from, to)
    }
    pub(crate) fn stage_directory(&self, path: &str) -> Result<StagedDirectory, TextFileError> {
        legacy::stage_directory_removal(&self.directory, &self.manuscript_id, path)
            .map(|path| StagedDirectory { path })
    }
    pub(crate) fn restore_directory(
        &self,
        path: &str,
        staged: &StagedDirectory,
    ) -> Result<(), TextFileError> {
        legacy::restore_staged_directory(&self.directory, &self.manuscript_id, path, &staged.path)
    }
    pub(crate) fn rename(
        &self,
        from_path: &str,
        to_path: &str,
        touch_destination: bool,
    ) -> Result<(), TextFileError> {
        legacy::rename_file(
            &self.directory,
            &self.manuscript_id,
            from_path,
            to_path,
            touch_destination,
        )
    }
    pub(crate) fn create(
        &self,
        relative_path: &str,
        content: &str,
    ) -> Result<String, TextFileError> {
        legacy::create_text_file_with_content(
            &self.directory,
            &self.manuscript_id,
            relative_path,
            content,
        )
    }
    pub(crate) fn check_identity(&self) -> Result<(), TextFileError> {
        legacy::manuscript_source_directory(&self.directory, &self.manuscript_id).map(|_| ())
    }
    pub(crate) fn new(directory: &std::path::Path, manuscript_id: &str) -> Self {
        Self {
            directory: directory.to_path_buf(),
            manuscript_id: manuscript_id.to_owned(),
        }
    }

    pub(crate) fn read(&self, relative_path: &str) -> Result<(String, String), TextFileError> {
        legacy::read_text_file(&self.directory, &self.manuscript_id, relative_path)
    }

    pub(crate) fn save(
        &self,
        relative_path: &str,
        content: &str,
        expected_sha256: Option<&str>,
    ) -> Result<String, TextFileError> {
        legacy::save_text_file(
            &self.directory,
            &self.manuscript_id,
            relative_path,
            content,
            expected_sha256,
        )
    }

    pub(crate) fn delete(&self, relative_path: &str) -> Result<(), TextFileError> {
        legacy::delete_file(&self.directory, &self.manuscript_id, relative_path)
    }

    /// Preserve the original raw existence observation; validation still occurs
    /// at the actual text read/write, rather than being added to this branch.
    pub(crate) fn exists(&self, relative_path: &str) -> Result<bool, TextFileError> {
        Ok(
            legacy::manuscript_source_directory(&self.directory, &self.manuscript_id)?
                .join(relative_path)
                .exists(),
        )
    }

    pub(crate) fn is_file(&self, relative_path: &str) -> Result<bool, TextFileError> {
        Ok(
            legacy::manuscript_source_directory(&self.directory, &self.manuscript_id)?
                .join(relative_path)
                .is_file(),
        )
    }

    /// Original symlink_metadata presence branch: only NotFound becomes absent.
    pub(crate) fn present(&self, relative_path: &str) -> Result<bool, TextFileError> {
        let source = legacy::manuscript_source_directory(&self.directory, &self.manuscript_id)?;
        match fs::symlink_metadata(source.join(relative_path)) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Metadata is observed only after the original file/path/symlink checks.
    /// The caller can fail closed without receiving the resolved absolute path.
    #[cfg(unix)]
    pub(crate) fn stat(&self, relative_path: &str) -> Result<FileStat, TextFileError> {
        use std::os::unix::fs::MetadataExt;
        let path =
            legacy::resolve_existing_file(&self.directory, &self.manuscript_id, relative_path)?;
        let metadata = fs::symlink_metadata(path)?;
        Ok(FileStat {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            mtime: metadata.mtime(),
            mtime_nsec: metadata.mtime_nsec(),
            ctime: metadata.ctime(),
            ctime_nsec: metadata.ctime_nsec(),
        })
    }
}

#[cfg(unix)]
pub(crate) struct FileStat {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
    pub mtime: i64,
    pub mtime_nsec: i64,
    pub ctime: i64,
    pub ctime_nsec: i64,
}
