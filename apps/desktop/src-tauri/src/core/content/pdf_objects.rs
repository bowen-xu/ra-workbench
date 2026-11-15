//! Fixed PDF content-object operations inside an already admitted SQL snapshot.
//! These adapters never open a connection, take a lock or own a transaction.
//! The caller retains its established content/DB admission and commit policy.
use std::{
    fs::{self, File},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
};

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum PdfObjectError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("invalid PDF source: {0}")]
    InvalidPdf(String),
    #[error("unsafe stored PDF object")]
    UnsafeStoredObject,
}

/// Rooted internal repository access. Fields and paths are never exposed.
/// Binding SQL is internal adapter assembly, not a plugin SQL capability.
#[derive(Clone, Copy)]
pub(crate) struct PdfObjectStore<'root> {
    directory: &'root Path,
}

pub(crate) struct PdfObjectsIn<'root, 'db> {
    directory: &'root Path,
    connection: &'db Connection,
}

/// A single source, either an existing user-selected path or Core-owned staging.
/// Staged inputs retain NamedTempFile RAII through the original SQL updates.
pub(crate) struct PdfImportSource {
    source: Source,
}

enum Source {
    File(PathBuf),
    Staged(tempfile::NamedTempFile),
}

impl PdfImportSource {
    fn path(&self) -> &Path {
        match &self.source {
            Source::File(path) => path,
            Source::Staged(staged) => staged.path(),
        }
    }
}

pub(crate) struct PdfProbe<'source> {
    source: &'source PdfImportSource,
    sha256: String,
    byte_size: u64,
}

/// Exact existing attachment metadata, not a root/path/file access capability.
/// Connector writes these original source labels before its existing URL/name
/// override inside the same transaction; changing them would alter that write.
pub(crate) struct PdfSourceMetadata {
    pub display_name: String,
    pub source_path: String,
}

impl PdfProbe<'_> {
    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(crate) fn source_metadata(&self) -> PdfSourceMetadata {
        let path = self.source.path();
        PdfSourceMetadata {
            display_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("document.pdf")
                .to_owned(),
            source_path: path.to_string_lossy().into_owned(),
        }
    }
}

pub(crate) struct PublishedPdfObject {
    pub sha256: String,
    pub byte_size: u64,
}

/// Value crossing the caller's transaction boundary. No connection is retained.
pub(crate) struct PdfPublicationCheckpoint {
    sha256: String,
    preexisting: bool,
}

impl<'root> PdfObjectStore<'root> {
    pub(super) fn for_directory(directory: &'root Path) -> Self {
        Self { directory }
    }

    #[cfg(test)]
    pub(crate) fn for_test(directory: &'root Path) -> Self {
        Self::for_directory(directory)
    }

    /// Internal native repository assembly over the caller's original snapshot.
    /// This method returns no connection and accepts no arbitrary SQL callback.
    pub(crate) fn in_snapshot<'db>(self, connection: &'db Connection) -> PdfObjectsIn<'root, 'db> {
        PdfObjectsIn {
            directory: self.directory,
            connection,
        }
    }

    /// No I/O or validation is moved before the domain's eligibility check.
    pub(crate) fn file_source(self, path: &Path) -> PdfImportSource {
        PdfImportSource {
            source: Source::File(path.to_owned()),
        }
    }

    /// Preserve Connector's old staging before the domain eligibility/header
    /// checks. Callers map staging failures to the original Connector error.
    pub(crate) fn stage_pdf_bytes(self, bytes: &[u8]) -> Result<PdfImportSource, PdfObjectError> {
        let mut staged = tempfile::NamedTempFile::new_in(self.directory)?;
        staged.write_all(bytes)?;
        Ok(PdfImportSource {
            source: Source::Staged(staged),
        })
    }

    pub(crate) fn checkpoint_pdf_bytes(self, bytes: &[u8]) -> PdfPublicationCheckpoint {
        let sha256 = format!("{:x}", Sha256::digest(bytes));
        let path = pdf_path(self.directory, &sha256);
        let preexisting = match fs::symlink_metadata(&path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => true,
        };
        PdfPublicationCheckpoint {
            sha256,
            preexisting,
        }
    }
}

impl PdfObjectsIn<'_, '_> {
    pub(crate) fn probe_pdf_source<'source>(
        &self,
        source: &'source PdfImportSource,
    ) -> Result<PdfProbe<'source>, PdfObjectError> {
        let path = source.path();
        let mut header = [0_u8; 5];
        File::open(path)?.read_exact(&mut header)?;
        if &header != b"%PDF-" {
            return Err(PdfObjectError::InvalidPdf(path.display().to_string()));
        }
        let (sha256, byte_size) = hash_file(path)?;
        Ok(PdfProbe {
            source,
            sha256,
            byte_size,
        })
    }

    /// Register inside the borrowed snapshot. Returning is not a commit receipt.
    pub(crate) fn publish_pdf(
        &self,
        proof: &PdfProbe<'_>,
    ) -> Result<PublishedPdfObject, PdfObjectError> {
        let relative_path = format!(
            "{}/{}/{}.pdf",
            &proof.sha256[0..2],
            &proof.sha256[2..4],
            proof.sha256
        );
        let destination = self.directory.join(&relative_path);
        if !destination.exists()
            && let Some(parent) = destination.parent()
        {
            fs::create_dir_all(parent)?;
            // Keep the original complete immutable publication protocol.
            let staged = tempfile::NamedTempFile::new_in(parent)?;
            fs::copy(proof.source.path(), staged.path())?;
            staged.as_file().sync_all()?;
            if let Err(error) = staged.persist_noclobber(&destination)
                && error.error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(error.error.into());
            }
        }
        if fs::symlink_metadata(&destination)?.file_type().is_symlink()
            || hash_file(&destination)? != (proof.sha256.clone(), proof.byte_size)
        {
            return Err(PdfObjectError::UnsafeStoredObject);
        }
        self.connection.execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
         values (?1, ?2, 'application/pdf', ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         on conflict(sha256) do nothing",
            params![proof.sha256, relative_path, proof.byte_size],
        )?;
        Ok(PublishedPdfObject {
            sha256: proof.sha256.clone(),
            byte_size: proof.byte_size,
        })
    }

    /// Connector's exact post-error cleanup policy. Ordinary imports never call
    /// this. Cleanup failures must not replace the original operation error.
    pub(crate) fn cleanup_unregistered_after_error(&self, checkpoint: &PdfPublicationCheckpoint) {
        let path = pdf_path(self.directory, &checkpoint.sha256);
        if !checkpoint.preexisting
            && matches!(
                self.connection.query_row(
                    "select exists(select 1 from content_objects where sha256=?1)",
                    [&checkpoint.sha256],
                    |row| row.get::<_, bool>(0),
                ),
                Ok(false)
            )
            && fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file())
            && hash_file(&path).is_ok_and(|(hash, _)| hash == checkpoint.sha256)
        {
            // Preserve the original failure if cleanup itself is unavailable.
            // A process crash before this branch remains an existing limitation.
            let _ = fs::remove_file(path);
        }
    }
}

fn pdf_path(directory: &Path, sha256: &str) -> PathBuf {
    directory
        .join(&sha256[..2])
        .join(&sha256[2..4])
        .join(format!("{sha256}.pdf"))
}

fn hash_file(path: &Path) -> Result<(String, u64), PdfObjectError> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PDF: &[u8] = b"%PDF-1.7\n% synthetic Core object fixture\n%%EOF\n";

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table content_objects(sha256 text primary key,relative_path text not null,mime_type text not null,byte_size integer not null,created_at text not null);").unwrap();
        db
    }

    fn source(root: &Path, name: &str) -> PdfImportSource {
        let path = root.join(name);
        fs::write(&path, PDF).unwrap();
        PdfImportSource {
            source: Source::File(path),
        }
    }

    #[test]
    fn publishes_exact_pdf_layout_and_reuses_existing_object_metadata() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("objects");
        let db = database();
        let store = PdfObjectStore::for_test(&directory);
        let input = source(root.path(), "selected.dat");
        let scoped = store.in_snapshot(&db);
        let proof = scoped.probe_pdf_source(&input).unwrap();
        let object = scoped.publish_pdf(&proof).unwrap();
        let relative: String = db
            .query_row(
                "select relative_path from content_objects where sha256=?1",
                [&object.sha256],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            relative,
            format!(
                "{}/{}/{}.pdf",
                &object.sha256[..2],
                &object.sha256[2..4],
                object.sha256
            )
        );
        assert_eq!(object.byte_size, PDF.len() as u64);
        assert_eq!(fs::read(directory.join(&relative)).unwrap(), PDF);
        db.execute(
            "update content_objects set created_at='original',mime_type='existing/type'",
            [],
        )
        .unwrap();
        scoped.publish_pdf(&proof).unwrap();
        let record: (String,String,i64) = db.query_row("select created_at,mime_type,(select count(*) from content_objects) from content_objects",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(record, ("original".into(), "existing/type".into(), 1));
    }

    #[test]
    fn staged_source_keeps_original_metadata_and_raii_until_the_consumer_finishes() {
        let root = tempfile::tempdir().unwrap();
        let db = database();
        let store = PdfObjectStore::for_test(root.path());
        let input = store.stage_pdf_bytes(PDF).unwrap();
        let metadata = store
            .in_snapshot(&db)
            .probe_pdf_source(&input)
            .unwrap()
            .source_metadata();
        let observed = PathBuf::from(&metadata.source_path);
        assert_eq!(observed.parent(), Some(root.path()));
        assert_eq!(
            observed.file_name().unwrap().to_str().unwrap(),
            metadata.display_name
        );
        assert_eq!(fs::read(&observed).unwrap(), PDF);
        // No path/NamedTempFile getter is supplied to the consumer: these are
        // only the exact two metadata values already written by the old flow.
        drop(input);
        assert!(!observed.exists());
    }

    #[test]
    fn borrowed_transaction_preserves_rollback_and_explicit_connector_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let mut db = database();
        let store = PdfObjectStore::for_test(root.path());
        let input = source(root.path(), "source.pdf");
        let checkpoint = store.checkpoint_pdf_bytes(PDF);
        let hash = format!("{:x}", Sha256::digest(PDF));
        {
            let tx = db.transaction().unwrap();
            let scoped = store.in_snapshot(&tx);
            let proof = scoped.probe_pdf_source(&input).unwrap();
            scoped.publish_pdf(&proof).unwrap();
            assert_eq!(
                tx.query_row("select count(*) from content_objects", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            tx.rollback().unwrap();
        }
        assert_eq!(
            db.query_row("select count(*) from content_objects", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let published = pdf_path(root.path(), &hash);
        assert!(
            published.exists(),
            "ordinary rollback does not add automatic file removal"
        );
        store
            .in_snapshot(&db)
            .cleanup_unregistered_after_error(&checkpoint);
        assert!(!published.exists());
        assert!(input.path().exists());
    }

    #[test]
    fn cleanup_preserves_preexisting_and_registered_objects() {
        let root = tempfile::tempdir().unwrap();
        let db = database();
        let store = PdfObjectStore::for_test(root.path());
        let input = source(root.path(), "source.pdf");
        let absent = store.checkpoint_pdf_bytes(PDF);
        let scoped = store.in_snapshot(&db);
        let proof = scoped.probe_pdf_source(&input).unwrap();
        let object = scoped.publish_pdf(&proof).unwrap();
        let path = pdf_path(root.path(), &object.sha256);
        scoped.cleanup_unregistered_after_error(&absent);
        assert!(path.exists(), "registered row blocks deletion");
        let existing = store.checkpoint_pdf_bytes(PDF);
        db.execute("delete from content_objects", []).unwrap();
        scoped.cleanup_unregistered_after_error(&existing);
        assert!(
            path.exists(),
            "preexisting file blocks deletion after rollback"
        );
    }

    #[test]
    fn cleanup_preserves_changed_bytes_and_swallows_database_lookup_failure() {
        let root = tempfile::tempdir().unwrap();
        let db = database();
        let store = PdfObjectStore::for_test(root.path());
        let checkpoint = store.checkpoint_pdf_bytes(PDF);
        let path = pdf_path(root.path(), &checkpoint.sha256);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"different bytes").unwrap();
        store
            .in_snapshot(&db)
            .cleanup_unregistered_after_error(&checkpoint);
        assert_eq!(fs::read(&path).unwrap(), b"different bytes");
        fs::write(&path, PDF).unwrap();
        db.execute_batch("drop table content_objects").unwrap();
        store
            .in_snapshot(&db)
            .cleanup_unregistered_after_error(&checkpoint);
        assert!(
            path.exists(),
            "cleanup failure never removes a file on uncertain registration"
        );
    }

    #[cfg(unix)]
    #[test]
    fn publication_and_cleanup_reject_symlinked_destinations() {
        let root = tempfile::tempdir().unwrap();
        let db = database();
        let store = PdfObjectStore::for_test(root.path());
        let checkpoint = store.checkpoint_pdf_bytes(PDF);
        let input = source(root.path(), "source.pdf");
        let path = pdf_path(root.path(), &checkpoint.sha256);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(input.path(), &path).unwrap();
        let scoped = store.in_snapshot(&db);
        let proof = scoped.probe_pdf_source(&input).unwrap();
        assert!(matches!(
            scoped.publish_pdf(&proof),
            Err(PdfObjectError::UnsafeStoredObject)
        ));
        scoped.cleanup_unregistered_after_error(&checkpoint);
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(input.path().exists());
    }

    #[test]
    fn probe_preserves_invalid_header_and_short_file_error_categories() {
        let root = tempfile::tempdir().unwrap();
        let db = database();
        let store = PdfObjectStore::for_test(root.path());
        let path = root.path().join("invalid.pdf");
        fs::write(&path, b"NOTPDF").unwrap();
        let input = store.file_source(&path);
        assert!(
            matches!(store.in_snapshot(&db).probe_pdf_source(&input),Err(PdfObjectError::InvalidPdf(source)) if source==path.display().to_string())
        );
        fs::write(&path, b"%P").unwrap();
        assert!(
            matches!(store.in_snapshot(&db).probe_pdf_source(&input),Err(PdfObjectError::Io(error)) if error.kind()==std::io::ErrorKind::UnexpectedEof)
        );
    }
}
