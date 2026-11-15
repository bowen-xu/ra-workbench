//! Closed Core facts and content capabilities for the literature reading policy.
use std::{fs, io::Read, path::Path};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{agent::instructions::directory::Directory, storage::AppStore};

use super::{error::AiError, llm_configuration, reading_contracts::LlmModelSelection};

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct ReadingSupportError(AiError);

impl ReadingSupportError {
    pub(crate) fn into_ai_error(self) -> AiError {
        self.0
    }
}

impl From<AiError> for ReadingSupportError {
    fn from(error: AiError) -> Self {
        Self(error)
    }
}

impl From<rusqlite::Error> for ReadingSupportError {
    fn from(error: rusqlite::Error) -> Self {
        Self(AiError::from(error))
    }
}

#[derive(Clone)]
pub(crate) struct ParseRunRecord {
    pub(crate) id: String,
    pub(crate) json_content: Value,
}

/// Borrows the caller's existing connection/guard without exposing it.
pub(crate) struct PaperReadingFacts<'db> {
    db: &'db Connection,
}

impl<'db> PaperReadingFacts<'db> {
    pub(crate) fn new(db: &'db Connection) -> Self {
        Self { db }
    }

    #[cfg(test)]
    pub(crate) fn latest_parse_run(
        &self,
        attachment_id: &str,
        content_sha256: &str,
    ) -> Result<Option<ParseRunRecord>, ReadingSupportError> {
        self.latest_parse_run_with_source(attachment_id, content_sha256)
            .map(|value| value.map(|(record, _)| record))
    }

    pub(crate) fn latest_parse_run_with_source(
        &self,
        attachment_id: &str,
        content_sha256: &str,
    ) -> Result<Option<(ParseRunRecord, String)>, ReadingSupportError> {
        self.db
            .query_row(
                "select id, json_content from docling_parse_runs
             where attachment_id = ?1 and content_sha256 = ?2
               and parse_status in ('success', 'partial_success')
               and json_content is not null
             order by created_at desc, finished_at desc limit 1",
                params![attachment_id, content_sha256],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .map(|(id, json_content)| {
                serde_json::from_str(&json_content)
                    .map(|value| {
                        (
                            ParseRunRecord {
                                id,
                                json_content: value,
                            },
                            json_content,
                        )
                    })
                    .map_err(|error| {
                        AiError::InvalidInput(format!("Docling JSON 无法解析：{error}")).into()
                    })
            })
            .transpose()
    }

    pub(crate) fn has_stale_parse_run(
        &self,
        attachment_id: &str,
    ) -> Result<bool, ReadingSupportError> {
        self.db
            .query_row(
                "select exists(
                select 1 from docling_parse_runs where attachment_id = ?1
                  and parse_status in ('success', 'partial_success') and json_content is not null
             )",
                [attachment_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub(crate) fn selection_for_route(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<LlmModelSelection>, ReadingSupportError> {
        llm_configuration::selection_for_route(self.db, kind, id).map_err(Into::into)
    }

    pub(crate) fn image_metadata(&self, hash: &str) -> Result<ImageMetadata, ReadingSupportError> {
        image_metadata(self.db, hash)
    }

    pub(crate) fn managed_snapshot_references(
        &self,
        snapshot: &str,
    ) -> Result<bool, ReadingSupportError> {
        let managed:bool=self.db.query_row("select exists(select 1 from sqlite_master where type='table' and name='structured_resource_field_sources')",[],|row|row.get(0))?;
        if !managed {
            return Ok(false);
        }
        Ok(self.db.query_row("select exists(select 1 from structured_resource_field_sources where source_snapshot_id=?1) or exists(select 1 from structured_resource_drafts where source_snapshot_id=?1) or exists(select 1 from agent_text_read_scopes where json_extract(snapshot_json,'$.kind')='structuredEvidenceRead' and json_extract(snapshot_json,'$.snapshotId')=?1)",[snapshot],|row|row.get(0))?)
    }
}

const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

pub(crate) struct StoredImageBytes {
    pub(crate) mime_type: String,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct ImageMetadata {
    pub(crate) mime_type: String,
    pub(crate) byte_size: u64,
}

/// Owns content metadata and file access; domain item identities stay outside.
pub(crate) struct PaperReadingImages<'a> {
    db: &'a Connection,
    store: &'a AppStore,
}

impl<'a> PaperReadingImages<'a> {
    pub(crate) fn new(db: &'a Connection, store: &'a AppStore) -> Self {
        Self { db, store }
    }

    pub(crate) fn save(&self, data_url: &str) -> Result<String, ReadingSupportError> {
        let encoded = data_url
            .strip_prefix("data:image/png;base64,")
            .ok_or_else(|| AiError::InvalidInput("图片截图必须是 PNG data URL".to_owned()))?;
        let bytes = BASE64
            .decode(encoded)
            .map_err(|_| AiError::InvalidInput("图片截图不是有效的 base64".to_owned()))?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(AiError::InvalidInput("单张结构化图片截图超过 20 MiB".to_owned()).into());
        }
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let relative_path = format!(
            "ai-reading-images/{}/{}/{}.png",
            &sha256[0..2],
            &sha256[2..4],
            sha256
        );
        let path = self.store.content_directory().join(&relative_path);
        {
            let _content_write = self.store.content_write().map_err(AiError::from)?;
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|_| AiError::TemporaryUnavailable)?;
                }
                fs::write(&path, &bytes).map_err(|_| AiError::TemporaryUnavailable)?;
            }
        }
        self.db.execute(
            "insert into content_objects (sha256, relative_path, mime_type, byte_size, created_at)
             values (?1, ?2, 'image/png', ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(sha256) do nothing",
            params![
                sha256,
                relative_path,
                i64::try_from(bytes.len()).unwrap_or(i64::MAX)
            ],
        )?;
        Ok(sha256)
    }

    pub(crate) fn read(&self, hash: &str) -> Result<StoredImageBytes, ReadingSupportError> {
        let metadata = self
            .db
            .query_row(
                "select relative_path,mime_type,byte_size from content_objects where sha256=?1",
                [hash],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<u64>>(2)?,
                    ))
                },
            )
            .optional()?
            .unwrap_or((None, None, None));
        let _content_read = self.store.content_read().map_err(AiError::from)?;
        let bytes = read_verified_image(
            self.store.content_directory(),
            hash,
            metadata.0.as_deref(),
            metadata.1.as_deref(),
            metadata.2,
        )?;
        Ok(StoredImageBytes {
            mime_type: "image/png".to_owned(),
            bytes,
        })
    }
}

fn image_metadata(db: &Connection, hash: &str) -> Result<ImageMetadata, ReadingSupportError> {
    db.query_row(
        "select mime_type,byte_size from content_objects where sha256=?1",
        [hash],
        |row| {
            Ok(ImageMetadata {
                mime_type: row.get(0)?,
                byte_size: row.get(1)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| AiError::TemporaryUnavailable.into())
}

fn invalid_image() -> AiError {
    AiError::InvalidInput("论文上下文图片缓存缺失或损坏；请重新建立结构化阅读".to_owned())
}

fn read_verified_image(
    root: &Path,
    hash: &str,
    relative: Option<&str>,
    mime: Option<&str>,
    size: Option<u64>,
) -> Result<Vec<u8>, AiError> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid_image());
    }
    let expected = format!(
        "ai-reading-images/{}/{}/{}.png",
        &hash[..2],
        &hash[2..4],
        hash
    );
    let size = size
        .filter(|s| *s > 0 && *s <= MAX_IMAGE_BYTES as u64)
        .ok_or_else(invalid_image)?;
    if relative != Some(expected.as_str()) || mime != Some("image/png") {
        return Err(invalid_image());
    }
    // Canonicalize only the application-owned root; every descendant is opened
    // relative to its retained handle, without following symlinks.
    let root = root.canonicalize().map_err(|_| invalid_image())?;
    let mut directory = Directory::root(&root).map_err(|_| invalid_image())?;
    for component in ["ai-reading-images", &hash[..2], &hash[2..4]] {
        directory = directory
            .child(Path::new(component))
            .map_err(|_| invalid_image())?;
    }
    let file = directory
        .config_file(&format!("{hash}.png"))
        .map_err(|_| invalid_image())?
        .ok_or_else(invalid_image)?;
    if file.metadata().map_err(|_| invalid_image())?.len() != size {
        return Err(invalid_image());
    }
    let mut bytes = Vec::new();
    file.take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid_image())?;
    if bytes.len() as u64 != size
        || format!("{:x}", Sha256::digest(&bytes)) != hash
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err(invalid_image());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_parse_run_requires_current_attachment_hash() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch(
                "create table docling_parse_runs(
                    id text primary key, attachment_id text, content_sha256 text,
                    parse_status text, json_content text, created_at text, finished_at text
                );",
            )
            .expect("schema");
        connection
            .execute(
                "insert into docling_parse_runs values ('old', 'att', ?1, 'success', '{}', '2', '2')",
                ["b".repeat(64)],
            )
            .expect("old parse run");
        assert!(
            PaperReadingFacts::new(&connection)
                .latest_parse_run("att", &"a".repeat(64))
                .expect("query")
                .is_none()
        );
    }

    #[test]
    fn verifies_exact_bytes_and_rejects_changed_metadata_or_content() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"\x89PNG\r\n\x1a\nsynthetic-content";
        let hash = format!("{:x}", Sha256::digest(bytes));
        let relative = format!(
            "ai-reading-images/{}/{}/{hash}.png",
            &hash[..2],
            &hash[2..4]
        );
        let path = dir.path().join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let load = |path, mime, size| read_verified_image(dir.path(), &hash, path, mime, size);
        assert_eq!(
            load(Some(&relative), Some("image/png"), Some(bytes.len() as u64)).unwrap(),
            bytes
        );
        for (p, m, s) in [
            (None, Some("image/png"), Some(1)),
            (Some("../image.png"), Some("image/png"), Some(1)),
            (
                Some(relative.as_str()),
                Some("image/jpeg"),
                Some(bytes.len() as u64),
            ),
            (
                Some(relative.as_str()),
                Some("image/png"),
                Some(MAX_IMAGE_BYTES as u64 + 1),
            ),
        ] {
            assert!(load(p, m, s).is_err());
        }
        fs::write(&path, vec![0; bytes.len()]).unwrap();
        assert!(load(Some(&relative), Some("image/png"), Some(bytes.len() as u64)).is_err());
        fs::remove_file(path).unwrap();
        assert!(load(Some(&relative), Some("image/png"), Some(bytes.len() as u64)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_files_and_directories() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"\x89PNG\r\n\x1a\ncontent";
        let hash = format!("{:x}", Sha256::digest(bytes));
        let relative = format!(
            "ai-reading-images/{}/{}/{hash}.png",
            &hash[..2],
            &hash[2..4]
        );
        let path = dir.path().join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let outside = dir.path().join("outside");
        fs::write(&outside, bytes).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(
            read_verified_image(
                dir.path(),
                &hash,
                Some(&relative),
                Some("image/png"),
                Some(bytes.len() as u64)
            )
            .is_err()
        );
        fs::remove_dir_all(dir.path().join("ai-reading-images")).unwrap();
        symlink(dir.path(), dir.path().join("ai-reading-images")).unwrap();
        assert!(
            read_verified_image(
                dir.path(),
                &hash,
                Some(&relative),
                Some("image/png"),
                Some(bytes.len() as u64)
            )
            .is_err()
        );
    }
}
