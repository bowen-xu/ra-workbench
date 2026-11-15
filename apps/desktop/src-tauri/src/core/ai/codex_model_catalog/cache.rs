//! Account- and protocol-scoped sanitized metadata; never stores credentials/prompts.
use super::{CatalogModel, MAX_BYTES};
use serde::{Deserialize, Serialize};
use std::{
    io::{Error, ErrorKind, Read},
    path::Path,
};

pub(super) const CURRENT_SCHEMA_VERSION: u8 = 2;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CatalogCache {
    pub schema_version: u8,
    pub client_version: String,
    pub fetched_at: u64,
    pub etag: Option<String>,
    pub models: Vec<CatalogModel>,
}
pub(super) fn read_bounded(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::new(ErrorKind::InvalidData, "catalog too large"));
    }
    Ok(bytes)
}
pub(super) fn read(path: &Path) -> Option<CatalogCache> {
    let cache: CatalogCache = serde_json::from_slice(&read_bounded(path).ok()?).ok()?;
    if !matches!(cache.schema_version, 1 | CURRENT_SCHEMA_VERSION)
        || !super::versions::is_version(&cache.client_version)
        || cache.fetched_at > 9_007_199_254_740_991
        || cache.etag.as_ref().is_some_and(|etag| {
            etag.len() > 1024 || reqwest::header::HeaderValue::from_str(etag).is_err()
        })
        || super::model::validate(&cache.models).is_err()
    {
        return None;
    }
    Some(cache)
}
pub(super) fn write(path: &Path, cache: &CatalogCache) -> std::io::Result<()> {
    write_json(path, cache)
}
pub(super) fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "cache path"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec(value).map_err(Error::other)?;
    std::fs::write(&temporary, bytes)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}
