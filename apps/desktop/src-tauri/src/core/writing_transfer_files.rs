//! Original physical transfer and external publication shared by both adapters.
use crate::{
    agent::writing_error::WritingError, export_file::publish_external_file, storage::AppStore,
};
use research_workbench_native_sdk::{
    writing_project::ManuscriptExportResult,
    writing_transfer::{SourceInspection, ZipEntry, ZipHeader},
};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub(crate) fn copy_directory(
    source: &Path,
    destination: &Path,
    inspection: &SourceInspection,
) -> Result<(), WritingError> {
    fs::create_dir_all(destination)?;
    for relative_path in &inspection.files {
        research_workbench_native_sdk::resource_path::validate_relative_path(relative_path)
            .map_err(|_| WritingError::InvalidPath)?;
        let source_path = source.join(relative_path);
        let metadata = fs::symlink_metadata(&source_path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(WritingError::SymlinkNotAllowed);
        }
        let target = destination.join(relative_path);
        fs::create_dir_all(target.parent().ok_or(WritingError::InvalidPath)?)?;
        let mut input = File::open(source_path)?;
        let mut output = File::options().create_new(true).write(true).open(target)?;
        let copied = std::io::copy(
            &mut Read::take(&mut input, metadata.len().saturating_add(1)),
            &mut output,
        )?;
        if copied != metadata.len() {
            return Err(WritingError::InvalidManuscript);
        }
        output.sync_all()?;
    }
    Ok(())
}

pub(crate) fn copy_zip(
    archive: &mut ZipArchive<File>,
    destination: &Path,
    entries: &[ZipEntry],
) -> Result<(), WritingError> {
    fs::create_dir_all(destination)?;
    for entry in entries {
        research_workbench_native_sdk::resource_path::validate_relative_path(&entry.relative_path)
            .map_err(|_| WritingError::InvalidPath)?;
        let mut input = archive.by_index(entry.archive_index)?;
        let target = destination.join(&entry.relative_path);
        fs::create_dir_all(target.parent().ok_or(WritingError::InvalidPath)?)?;
        let mut output = File::options().create_new(true).write(true).open(target)?;
        let copied = std::io::copy(
            &mut input.by_ref().take(entry.byte_size.saturating_add(1)),
            &mut output,
        )?;
        if copied != entry.byte_size {
            return Err(WritingError::InvalidManuscript);
        }
        output.sync_all()?;
    }
    Ok(())
}

pub(crate) fn verify_zip_headers(
    archive: &mut ZipArchive<File>,
    expected: &[ZipHeader],
) -> Result<(), WritingError> {
    if archive.len() != expected.len() {
        return Err(WritingError::InvalidManuscript);
    }
    for (index, expected) in expected.iter().enumerate() {
        let file = archive.by_index(index)?;
        let current = ZipHeader {
            name: file.name().into(),
            size: file.size(),
            compressed_size: file.compressed_size(),
            crc32: file.crc32(),
            is_dir: file.is_dir(),
            is_file: file.is_file(),
            is_symlink: file.is_symlink(),
            encrypted: file.encrypted(),
        };
        if current != *expected {
            return Err(WritingError::InvalidManuscript);
        }
    }
    Ok(())
}

/// Retry only name collisions; the exporter still atomically refuses overwrites.
pub(crate) fn export_numbered(
    destination: &str,
    rename_on_conflict: bool,
    mut export: impl FnMut(&str) -> Result<ManuscriptExportResult, WritingError>,
) -> Result<ManuscriptExportResult, WritingError> {
    let path = Path::new(destination);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(WritingError::InvalidPath)?;
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .ok_or(WritingError::InvalidPath)?;
    for index in 0..10_000 {
        let candidate = if index == 0 {
            path.to_owned()
        } else {
            path.with_file_name(format!("{stem} ({index}).{extension}"))
        };
        match export(&candidate.to_string_lossy()) {
            Err(WritingError::FileConflict) if rename_on_conflict => continue,
            Err(WritingError::Io(error))
                if rename_on_conflict && error.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                continue;
            }
            result => return result,
        }
    }
    Err(WritingError::FileConflict)
}

pub(crate) fn export_source(
    source: &Path,
    destination: &Path,
    files: &[String],
) -> Result<ManuscriptExportResult, WritingError> {
    for path in files {
        research_workbench_native_sdk::resource_path::validate_relative_path(path)
            .map_err(|_| WritingError::InvalidPath)?;
    }
    let temporary = temporary_sibling(destination);
    let result = (|| {
        let file = File::options()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let mut archive = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let mut total_bytes = 0_u64;
        for relative_path in files {
            let bytes = fs::read(source.join(relative_path))?;
            total_bytes += bytes.len() as u64;
            archive.start_file(relative_path.replace('\\', "/"), options)?;
            archive.write_all(&bytes)?;
        }
        let file = archive.finish()?;
        file.sync_all()?;
        publish_external_file(&temporary, destination)?;
        Ok::<_, WritingError>((files.len(), total_bytes))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    let (file_count, byte_size) = result?;
    Ok(ManuscriptExportResult {
        destination_path: destination.to_string_lossy().into_owned(),
        file_count,
        byte_size,
    })
}

pub(crate) fn validated_destination(value: &str, extension: &str) -> Result<PathBuf, WritingError> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute()
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(extension))
    {
        return Err(WritingError::InvalidPath);
    }
    if path.exists() {
        return Err(WritingError::FileConflict);
    }
    let parent = path.parent().ok_or(WritingError::InvalidPath)?;
    if !parent.is_dir() {
        return Err(WritingError::InvalidPath);
    }
    let filename = path.file_name().ok_or(WritingError::InvalidPath)?;
    Ok(parent.canonicalize()?.join(filename))
}

pub(crate) fn reject_managed_destination(
    store: &AppStore,
    path: &Path,
) -> Result<(), WritingError> {
    let manuscripts = canonicalize_with_missing_leaf(store.manuscript_directory())?;
    let content = canonicalize_with_missing_leaf(store.content_directory())?;
    let cache = canonicalize_with_missing_leaf(store.writing_cache_directory())?;
    if path.starts_with(manuscripts) || path.starts_with(content) || path.starts_with(cache) {
        return Err(WritingError::InvalidPath);
    }
    Ok(())
}

fn canonicalize_with_missing_leaf(path: &Path) -> Result<PathBuf, WritingError> {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        missing.push(existing.file_name().ok_or(WritingError::InvalidPath)?);
        existing = existing.parent().ok_or(WritingError::InvalidPath)?;
    }
    let mut result = existing.canonicalize()?;
    for component in missing.into_iter().rev() {
        result.push(component);
    }
    Ok(result)
}

fn temporary_sibling(destination: &Path) -> PathBuf {
    destination
        .parent()
        .unwrap_or(destination)
        .join(format!(".writing-export-{}.tmp", Uuid::new_v4()))
}

pub(crate) fn write_external_atomically(
    destination: &Path,
    bytes: &[u8],
) -> Result<(), WritingError> {
    crate::export_file::write_new_file(destination, bytes).map_err(Into::into)
}

pub(crate) fn export_pdf_bytes(
    destination: &Path,
    bytes: &[u8],
) -> Result<ManuscriptExportResult, WritingError> {
    if !bytes.starts_with(b"%PDF-") {
        return Err(WritingError::Process("编译产物不是有效 PDF".to_owned()));
    }
    write_external_atomically(destination, bytes)?;
    Ok(ManuscriptExportResult {
        destination_path: destination.to_string_lossy().into_owned(),
        file_count: 1,
        byte_size: bytes.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_changed_archive_headers_before_copying_members() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.zip");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file("paper/main.tex", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"\\documentclass{article}").unwrap();
        zip.finish().unwrap();
        let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
        let header = {
            let file = archive.by_index(0).unwrap();
            ZipHeader {
                name: file.name().into(),
                size: file.size(),
                compressed_size: file.compressed_size(),
                crc32: file.crc32(),
                is_dir: file.is_dir(),
                is_file: file.is_file(),
                is_symlink: file.is_symlink(),
                encrypted: file.encrypted(),
            }
        };
        verify_zip_headers(&mut archive, std::slice::from_ref(&header)).unwrap();
        assert!(verify_zip_headers(&mut archive, &[]).is_err());
        let mut changed = header.clone();
        changed.crc32 ^= 1;
        assert!(verify_zip_headers(&mut archive, &[changed]).is_err());
        let mut changed = header.clone();
        changed.compressed_size += 1;
        assert!(verify_zip_headers(&mut archive, &[changed]).is_err());
        let mut changed = header;
        changed.name = "paper/other.tex".into();
        assert!(verify_zip_headers(&mut archive, &[changed]).is_err());
        assert!(!root.path().join("destination").exists());
    }
}
