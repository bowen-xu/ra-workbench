//! Page entrances carry lifecycle in the existing document transaction. Paste
//! restores the same resource; ordinary links never acquire lifecycle authority.
use super::{NoteDocument, NoteDocumentBlock, error::NotesError};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeSet;

pub(crate) fn targets(blocks: &[NoteDocumentBlock]) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for block in blocks {
        if (block.block_type == "childPage"
            || (block.block_type == "researchObjectReference" && block.props["pageEntry"] == true))
            && block.content["kind"] == "reference"
            && block.content["targetKind"] == "note_page"
            && let Some(target) = block.content["targetId"].as_str()
        {
            result.insert(target.into());
        }
        result.extend(targets(&block.children));
    }
    result
}
fn legacy<'a>(blocks: &'a [NoteDocumentBlock], target: &str) -> Option<&'a NoteDocumentBlock> {
    blocks.iter().find_map(|b| {
        if b.block_type == "childPage" && b.content["targetId"] == target {
            Some(b)
        } else {
            legacy(&b.children, target)
        }
    })
}
fn held_elsewhere(tx: &Transaction<'_>, source: &str, target: &str) -> Result<bool, NotesError> {
    Ok(tx.query_row(
        "select exists(select 1 from note_blocks b join note_pages p on p.id=b.page_id
         where b.page_id<>?1 and b.scope_kind='page' and b.status='active' and p.status='active'
         and json_extract(b.content_json,'$.kind')='reference' and json_extract(b.content_json,'$.targetKind')='note_page'
         and json_extract(b.content_json,'$.targetId')=?2
         and (b.block_type='childPage' or (b.block_type='researchObjectReference' and json_extract(b.props_json,'$.pageEntry')=1)))",
        params![source, target], |row| row.get(0),
    )?)
}
fn status(
    tx: &Transaction<'_>,
    id: &str,
    desired: &str,
    parent: Option<Option<String>>,
) -> Result<(), NotesError> {
    let before = super::text_change_set::read(tx, id)?;
    let parent = parent.unwrap_or_else(|| before.parent_id.clone());
    if before.status == desired && before.parent_id == parent {
        return Ok(());
    }
    tx.execute(
        "update note_pages set status=?2,parent_page_id=?3,
        trashed_at=case when ?2='trashed' then strftime('%Y-%m-%dT%H:%M:%fZ','now') else null end,
        revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1",
        params![id, desired, parent],
    )?;
    let after = super::text_change_set::read(tx, id)?;
    // Adopt acknowledged metadata through the common proposal owner. Neither
    // an entrance edit nor a lifecycle update accepts unrelated body text.
    crate::agent::text_tools::note_review::synchronize_metadata(tx, &before, &after)
        .map_err(NotesError::InvalidInput)
}
fn graph(
    tx: &Transaction<'_>,
    id: &str,
    source: &str,
    depth: usize,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<String>,
) -> Result<(), NotesError> {
    if id == source {
        return Err(NotesError::InvalidInput(
            "页面入口不能形成生命周期环路".into(),
        ));
    }
    if depth > 64 || seen.len() > 20_000 {
        return Err(NotesError::InvalidInput("页面入口结构超过安全限制".into()));
    }
    if !seen.insert(id.into()) {
        return Ok(());
    }
    let exists: bool = tx.query_row(
        "select exists(select 1 from note_pages where id=?1)",
        [id],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    let document = super::load_document_value(tx, id)?;
    out.push(id.into());
    for child in targets(&document.blocks) {
        graph(tx, &child, source, depth + 1, seen, out)?;
    }
    Ok(())
}
pub(super) fn reconcile(
    tx: &Transaction<'_>,
    before: &NoteDocument,
    after: &NoteDocument,
) -> Result<(), NotesError> {
    let old = targets(&before.blocks);
    let new = targets(&after.blocks);
    if new.contains(&after.page_id) {
        return Err(NotesError::InvalidInput("页面入口不能指向自身".into()));
    }
    for id in new.difference(&old) {
        let current: Option<(String, Option<String>)> = tx
            .query_row(
                "select status,parent_page_id from note_pages where id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current, parent)) = current else {
            return Err(NotesError::InvalidInput("页面入口目标不存在".into()));
        };
        let mut tree = vec![];
        graph(tx, id, &after.page_id, 0, &mut BTreeSet::new(), &mut tree)?;
        // A historical native history item can restore its exact legacy owner.
        // Clipboard entrances use the modern, independent-page representation.
        let parent_override = if let Some(block) = legacy(&after.blocks, id) {
            let proven:bool=tx.query_row("select exists(select 1 from note_blocks where id=?1 and page_id=?2 and block_type='childPage' and json_extract(content_json,'$.targetId')=?3)",params![block.id,after.page_id,id],|r|r.get(0))?;
            (proven && parent.as_deref() != Some(&after.page_id))
                .then(|| Some(after.page_id.clone()))
        } else {
            (current == "trashed").then_some(None)
        };
        if current == "trashed" || parent_override.is_some() {
            status(tx, id, "active", parent_override)?;
        }
        if current == "trashed" {
            for child in tree.into_iter().skip(1) {
                let state: String =
                    tx.query_row("select status from note_pages where id=?1", [&child], |r| {
                        r.get(0)
                    })?;
                if state == "trashed" {
                    status(tx, &child, "active", None)?;
                }
            }
        }
    }
    for id in old.difference(&new) {
        let current: String =
            tx.query_row("select status from note_pages where id=?1", [id], |r| {
                r.get(0)
            })?;
        // An enclosing metadata command may already have applied the lifecycle
        // and captured its inverse. Removing its entrance does not replay it.
        if current == "trashed" {
            continue;
        }
        if held_elsewhere(tx, &after.page_id, id)? {
            let parent: Option<String> = tx.query_row(
                "select parent_page_id from note_pages where id=?1",
                [id],
                |r| r.get(0),
            )?;
            if parent.as_deref() == Some(&after.page_id) {
                let state: String =
                    tx.query_row("select status from note_pages where id=?1", [id], |r| {
                        r.get(0)
                    })?;
                status(tx, id, &state, Some(None))?;
            }
            continue;
        }
        let mut tree = vec![];
        graph(tx, id, &after.page_id, 0, &mut BTreeSet::new(), &mut tree)?;
        for child in tree {
            if !new.contains(&child) && !held_elsewhere(tx, &after.page_id, &child)? {
                status(tx, &child, "trashed", None)?;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "page_entry_lifecycle/tests.rs"]
mod tests;
