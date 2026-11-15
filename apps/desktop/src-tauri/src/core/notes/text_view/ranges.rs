//! Use the patch engine's exact byte ranges, never a whole-page identity guess.
use super::{Result, parse, reconcile, render, render_block};
use crate::notes::{NoteDocument, NoteDocumentBlock};
use research_agent_core::text_patch::TextEdit;

fn mapped(offset: usize, edits: &[TextEdit], end: bool) -> Result<usize> {
    let mut shift = 0isize;
    for edit in edits {
        if edit.end < offset || (edit.end == offset && (end || edit.start != edit.end)) {
            shift += edit.replacement.len() as isize - (edit.end - edit.start) as isize;
        } else if edit.start < offset {
            return Err("block boundary was removed; widen the patch context".into());
        }
    }
    offset
        .checked_add_signed(shift)
        .ok_or_else(|| "invalid patch range".into())
}
pub(crate) fn candidate(
    before: &NoteDocument,
    title: &str,
    original: &str,
    source: &str,
    edits: &[TextEdit],
) -> Result<(String, NoteDocument)> {
    if render(before, title)? != original {
        return Err("note mapping snapshot changed; re-read".into());
    }
    let (title, parsed) = parse(&before.page_id, source)?;
    if before.blocks.is_empty() {
        return Ok((title, parsed));
    }
    let body = original.find("\n---\n").ok_or("invalid mapping header")? + 6;
    let mut spans = Vec::new();
    let mut cursor = body;
    for block in &before.blocks {
        let text = render_block(block)?;
        let end = (cursor + text.len() + 2).min(original.len());
        spans.push((cursor, end));
        cursor = end;
    }
    let body_edits = edits
        .iter()
        .filter(|edit| edit.end >= body)
        .collect::<Vec<_>>();
    if body_edits.is_empty() {
        return Ok((title, before.clone()));
    }
    if body_edits.iter().any(|edit| edit.start < body) {
        return Err("edit title and body in separate patch chunks".into());
    }
    let affected = spans
        .iter()
        .enumerate()
        .filter(|(_, (start, end))| {
            body_edits
                .iter()
                .any(|edit| edit.start <= *end && edit.end >= *start)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let first = *affected
        .first()
        .ok_or("patch range is outside the note body")?;
    let last = *affected.last().unwrap();
    let start = mapped(spans[first].0, edits, false)?.min(source.len());
    let end = mapped(spans[last].1, edits, true)?.min(source.len());
    let fragment = source.get(start..end).ok_or("invalid note mapping range")?;
    let mut changed = super::parse::parse_blocks(fragment, 0)?;
    reconcile::reconcile(&before.blocks[first..=last], &mut changed)?;
    let blocks: Vec<NoteDocumentBlock> = before.blocks[..first]
        .iter()
        .cloned()
        .chain(changed)
        .chain(before.blocks[last + 1..].iter().cloned())
        .collect();
    let candidate = NoteDocument {
        blocks,
        ..before.clone()
    };
    // Fragment parsing must yield exactly the same visible semantics as the full view.
    if candidate
        .blocks
        .iter()
        .map(render_block)
        .collect::<Result<Vec<_>>>()?
        != parsed
            .blocks
            .iter()
            .map(render_block)
            .collect::<Result<Vec<_>>>()?
    {
        return Err(
            "patch changed block boundaries outside its mapped range; expand the patch".into(),
        );
    }
    super::validate_candidate(&candidate, before)?;
    Ok((title, candidate))
}
