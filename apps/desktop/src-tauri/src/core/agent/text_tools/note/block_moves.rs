//! Recover exact complete-block moves across the atomic note group.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
fn collect(blocks: &[NoteDocumentBlock], output: &mut BTreeMap<String, NoteDocumentBlock>) {
    for block in blocks {
        output.insert(block.id.clone(), block.clone());
        collect(&block.children, output);
    }
}
pub(super) fn resolve(group: &mut [PendingNote]) -> Result<(), String> {
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    for note in &*group {
        if let Some(page) = &note.baseline {
            collect(&page.document.blocks, &mut before);
        }
        if let Some(page) = &note.candidate {
            collect(&page.document.blocks, &mut after);
        }
    }
    let mut removed: BTreeMap<String, Vec<NoteDocumentBlock>> = BTreeMap::new();
    for (id, block) in &before {
        if !after.contains_key(id) && block.block_type != "childPage" {
            removed
                .entry(crate::notes::text_view::render_block(block)?)
                .or_default()
                .push(block.clone());
        }
    }
    let mut additions: BTreeMap<String, usize> = BTreeMap::new();
    for (id, block) in &after {
        if !before.contains_key(id) {
            *additions
                .entry(crate::notes::text_view::render_block(block)?)
                .or_default() += 1;
        }
    }
    let mut moved = BTreeSet::new();
    fn visit(
        blocks: &mut [NoteDocumentBlock],
        before: &BTreeMap<String, NoteDocumentBlock>,
        removed: &BTreeMap<String, Vec<NoteDocumentBlock>>,
        additions: &BTreeMap<String, usize>,
        moved: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        for block in blocks {
            if !before.contains_key(&block.id) {
                let text = crate::notes::text_view::render_block(block)?;
                if let Some(matches) = removed.get(&text) {
                    if matches.len() != 1 || additions.get(&text) != Some(&1) {
                        return Err("complete block move is ambiguous; narrow the patch".into());
                    }
                    let original = &matches[0];
                    if !moved.insert(original.id.clone()) {
                        return Err("duplicate block move".into());
                    }
                    *block = original.clone();
                    continue;
                }
                if block.block_type == "unsupported" {
                    return Err(
                        "invented or copied opaque placeholder; move it with its original deletion"
                            .into(),
                    );
                }
            }
            visit(&mut block.children, before, removed, additions, moved)?;
        }
        Ok(())
    }
    for note in &mut *group {
        if let Some(page) = &mut note.candidate {
            visit(
                &mut page.document.blocks,
                &before,
                &removed,
                &additions,
                &mut moved,
            )?;
        }
    }
    fn unique(blocks: &[NoteDocumentBlock], ids: &mut BTreeSet<String>) -> Result<(), String> {
        for block in blocks {
            if !ids.insert(block.id.clone()) {
                return Err(
                    "block identity appears in multiple candidates; refuse an ambiguous move"
                        .into(),
                );
            }
            unique(&block.children, ids)?;
        }
        Ok(())
    }
    let mut ids = BTreeSet::new();
    for note in group {
        if let Some(page) = &note.candidate {
            unique(&page.document.blocks, &mut ids)?;
        }
    }
    Ok(())
}
