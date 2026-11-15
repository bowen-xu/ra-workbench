//! Stable attachment identity resolves only through the current authorized directory.
use super::*;

pub(super) fn schema() -> Value {
    json!({"type":"string","minLength":1,"maxLength":1024,"description":"Unified file ID from the source prompt/directory. Reads the file's current structured content; no local path or separate parse ID is needed. If multiple page scopes exist, use their listed sourceId instead."})
}
pub(super) fn validate(
    source: Option<&str>,
    file: Option<&str>,
    required: bool,
) -> Result<(), RunError> {
    if (source.is_some() && file.is_some()) || (required && source.is_none() && file.is_none()) {
        return Err(RunError::ToolArguments("Provide exactly one of fileId or sourceId for reading; search may omit both to search the authorized directory.".into()));
    }
    if [source, file]
        .into_iter()
        .flatten()
        .any(|id| id.trim().is_empty() || id.len() > 1024 || id.contains('\0'))
    {
        return Err(RunError::ToolArguments("fileId/sourceId must be a non-empty authorized ID, at most 1024 UTF-8 bytes, without NUL.".into()));
    }
    Ok(())
}
pub(super) fn id(entry: &Entry) -> Option<&str> {
    match &entry.resource {
        Some(Resource::Paper {
            attachment_id: Some(id),
            ..
        }) => Some(id),
        _ => None,
    }
}
pub(super) fn find<'a>(
    entries: &'a [Entry],
    source: Option<&str>,
    file: Option<&str>,
) -> Result<&'a Entry, Error> {
    let mut matches = entries.iter().filter(|entry| {
        source.is_some_and(|id| entry.identity.source_id == id)
            || file.is_some_and(|file| id(entry) == Some(file))
    });
    let entry = matches.next().ok_or(Error::SourceUnavailable)?;
    if matches.next().is_some() {
        return Err(Error::InvalidInput);
    }
    Ok(entry)
}
