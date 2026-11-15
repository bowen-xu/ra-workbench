use rusqlite::{Transaction, params};

use super::{
    error::NotesError,
    model::{ReadingNoteBlock, SourceRect},
};

/// Move only an unquoted positional annotation on its original, current PDF page.
pub(super) fn move_in(
    transaction: &Transaction<'_>,
    current: &ReadingNoteBlock,
    position: SourceRect,
) -> Result<(), NotesError> {
    if current.note_type != "annotation"
        || !current.source.quote_exact.is_empty()
        || current
            .source
            .rects
            .as_array()
            .is_none_or(|rects| rects.len() != 1)
        || current.source.state != "exact"
        || ![position.x, position.y, position.width, position.height]
            .iter()
            .all(|value| value.is_finite())
        || position.x < 0.0
        || position.y < 0.0
        || position.width <= 0.0
        || position.height <= 0.0
        || position.x + position.width > 1.0
        || position.y + position.height > 1.0
    {
        return Err(NotesError::InvalidInput(
            "只能移动当前附件中有效的位置笔记".to_owned(),
        ));
    }
    let rects = serde_json::to_string(&vec![position.clone()])
        .map_err(|error| NotesError::InvalidInput(error.to_string()))?;
    let mut selector = current.source.selector.clone();
    if let Some(selector) = selector.as_mut() {
        for segment in &mut selector.segments {
            if segment.page_index == current.source.page_index {
                segment.rects = vec![position.clone()];
            }
        }
    }
    let selector_json = selector
        .map(|value| serde_json::to_string(&value))
        .transpose()
        .map_err(|error| NotesError::InvalidInput(error.to_string()))?;
    let changed = transaction.execute("update source_anchors set rects_json=?2, selector_json=?3 where id=?1 and attachment_id=?4 and content_sha256=?5 and page_index=?6",
        params![current.source.anchor_id, rects, selector_json, current.source.attachment_id, current.source.content_sha256, current.source.page_index])?;
    if changed != 1 {
        return Err(NotesError::InvalidInput(
            "位置笔记的来源锚点已失效".to_owned(),
        ));
    }
    transaction.execute(
        "update reading_note_sources set rects_json=?2, selector_json=?3 where page_id=?1",
        params![current.page_id, rects, selector_json],
    )?;
    Ok(())
}
