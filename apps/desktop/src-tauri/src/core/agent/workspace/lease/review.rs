use super::change_bytes::read_version;
use super::*;
use research_agent_core::workspace::FileVersion;

const MAX_TEXT_BYTES: u64 = 1024 * 1024;
#[derive(Debug, PartialEq, Eq)]
pub enum ReviewContent {
    Text(String),
    Binary,
    TooLarge,
}
#[derive(Debug, PartialEq, Eq)]
pub struct ChangeContent {
    pub change: FileChange,
    /// None means an absent side of an addition/deletion, not an empty file.
    pub before: Option<ReviewContent>,
    pub after: Option<ReviewContent>,
}
impl WorkspaceLease {
    /// Read only the exact version previously selected for review. This is not
    /// an apply capability. The caller must retain exclusive, quiescent ownership.
    pub fn change_content(&self, expected: &FileChange) -> Result<ChangeContent, LeaseError> {
        if !self.changes()?.iter().any(|change| change == expected) {
            return Err(LeaseError::Conflict);
        }
        let before = expected
            .before
            .as_ref()
            .map(|version| read(&self.base, &expected.path, version))
            .transpose()?;
        let after = expected
            .after
            .as_ref()
            .map(|version| read(&self.work, &expected.path, version))
            .transpose()?;
        // Detect removed/replaced paths or writes during reading, including the
        // absent side. Never pair old inventory metadata with newer bytes.
        if !self.changes()?.iter().any(|change| change == expected) {
            return Err(LeaseError::Conflict);
        }
        Ok(ChangeContent {
            change: expected.clone(),
            before,
            after,
        })
    }
}
fn read(root: &Directory, path: &str, expected: &FileVersion) -> Result<ReviewContent, LeaseError> {
    research_agent_core::workspace::validate_path(path)?;
    if expected.byte_size > MAX_TEXT_BYTES {
        return Ok(ReviewContent::TooLarge);
    }
    let bytes = read_version(root, path, expected, MAX_TEXT_BYTES)?;
    if bytes.contains(&0) {
        return Ok(ReviewContent::Binary);
    }
    Ok(match String::from_utf8(bytes) {
        Ok(text) => ReviewContent::Text(text),
        Err(_) => ReviewContent::Binary,
    })
}
