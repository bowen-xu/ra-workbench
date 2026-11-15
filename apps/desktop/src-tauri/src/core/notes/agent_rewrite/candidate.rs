use super::{
    ContextText, InlineRewrite, NoteDocument, decode_source, find_mut, preview_frozen,
    validate_document,
};

/// Assemble a full-page candidate from native frozen evidence and a fresh full
/// page snapshot. This is pure validation, not acceptance or permission to save.
/// The caller must hold the revision/authorization boundary through persistence.
pub(crate) fn candidate(
    source: &ContextText,
    current_page: &ContextText,
    edits: &[InlineRewrite],
) -> Result<NoteDocument, &'static str> {
    let (frozen, selection) = decode_source(source)?;
    let (current, current_anchor) = decode_source(current_page)?;
    if current_anchor.block_id.is_some()
        || current.page_id != frozen.page_id
        || current.page_revision != frozen.page_revision
        || current.title != frozen.title
    {
        return Err("full page identity or revision changed");
    }
    let original_blocks = serde_json::to_value(&frozen.blocks).map_err(|_| "invalid baseline")?;
    let mut document = NoteDocument {
        title: None,
        schema_version: 1,
        page_id: current.page_id.to_string(),
        blocks: current.blocks,
    };
    validate_document(&document).map_err(|_| "invalid current page")?;
    let selected_blocks = match selection.block_id {
        Some(ref id) => {
            let block = find_mut(&mut document.blocks, id).ok_or("selected block removed")?;
            std::slice::from_ref(&*block)
        }
        None => document.blocks.as_slice(),
    };
    if serde_json::to_value(selected_blocks).map_err(|_| "invalid current selection")?
        != original_blocks
    {
        return Err("selected source changed");
    }
    // Validate every edit before constructing any replacement. Only block content
    // is copied; a selected subtree must never become the complete saved page.
    for change in preview_frozen(frozen, edits)? {
        let block = find_mut(&mut document.blocks, &change.block_id).ok_or("target removed")?;
        if block.content != change.before {
            return Err("block baseline changed");
        }
        block.content = change.after;
    }
    // A selected source can fit the budget while the full replacement cannot.
    validate_document(&document).map_err(|_| "invalid full replacement")?;
    Ok(document)
}

#[cfg(test)]
#[path = "candidate/tests.rs"]
mod tests;
