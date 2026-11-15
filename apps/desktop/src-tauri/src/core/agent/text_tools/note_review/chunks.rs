//! Stable block identities locate rich review fragments, never model text matching.
use crate::notes::{NoteDocument, NoteDocumentBlock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub id: String,
    pub before_block_ids: Vec<String>,
    pub after_block_ids: Vec<String>,
    pub before_index: usize,
    pub after_index: usize,
}
#[derive(Clone, Copy)]
struct Range {
    before_start: usize,
    before_end: usize,
    after_start: usize,
    after_end: usize,
}
fn same(before: &NoteDocumentBlock, after: &NoteDocumentBlock) -> bool {
    serde_json::to_value(before).ok() == serde_json::to_value(after).ok()
}
pub(super) fn equal(before: &NoteDocument, after: &NoteDocument) -> bool {
    // Title is document content in HBS. Resource metadata is already published
    // and must not create an independent Keep/Undo approval step.
    before.page_id == after.page_id
        && before.schema_version == after.schema_version
        && before.title == after.title
        && serde_json::to_value(&before.blocks).ok() == serde_json::to_value(&after.blocks).ok()
}
pub(in crate::agent::text_tools) fn empty(page: &str) -> NoteDocument {
    NoteDocument {
        title: None,
        schema_version: 1,
        page_id: page.into(),
        blocks: vec![],
    }
}
fn ranges(before: &NoteDocument, after: &NoteDocument) -> Vec<Range> {
    let positions = after
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.as_str(), i))
        .collect::<BTreeMap<_, _>>();
    let mut ranges = vec![];
    let (mut b, mut a) = (0, 0);
    for (index, block) in before.blocks.iter().enumerate() {
        if let Some(&target) = positions.get(block.id.as_str())
            && target >= a
            && same(block, &after.blocks[target])
        {
            if b != index || a != target {
                ranges.push(Range {
                    before_start: b,
                    before_end: index,
                    after_start: a,
                    after_end: target,
                });
            }
            b = index + 1;
            a = target + 1;
        }
    }
    if b != before.blocks.len() || a != after.blocks.len() {
        ranges.push(Range {
            before_start: b,
            before_end: before.blocks.len(),
            after_start: a,
            after_end: after.blocks.len(),
        });
    }
    // A moved identity joins its removal and insertion; accepting either fragment
    // alone would duplicate the identity. Bounding ranges remain disjoint.
    loop {
        let mut owners = BTreeMap::new();
        let mut join = None;
        for (i, range) in ranges.iter().enumerate() {
            for block in before.blocks[range.before_start..range.before_end]
                .iter()
                .chain(&after.blocks[range.after_start..range.after_end])
            {
                if let Some(prior) = owners.insert(block.id.as_str(), i)
                    && prior != i
                {
                    join = Some((prior, i));
                    break;
                }
            }
            if join.is_some() {
                break;
            }
        }
        let Some((first, last)) = join else {
            return ranges;
        };
        let merged = Range {
            before_start: ranges[first].before_start,
            before_end: ranges[last].before_end,
            after_start: ranges[first].after_start,
            after_end: ranges[last].after_end,
        };
        ranges.splice(first..=last, [merged]);
    }
}
pub(in crate::agent::text_tools) fn chunks(
    before: &NoteDocument,
    after: &NoteDocument,
) -> Result<Vec<Chunk>, String> {
    ranges(before, after)
        .into_iter()
        .map(|range| {
            let bytes = serde_json::to_vec(&(
                &before.blocks[range.before_start..range.before_end],
                &after.blocks[range.after_start..range.after_end],
                range.before_start,
                range.after_start,
            ))
            .map_err(|e| e.to_string())?;
            Ok(Chunk {
                id: format!("note-chunk:{:x}", Sha256::digest(bytes)),
                before_index: range.before_start,
                after_index: range.after_start,
                before_block_ids: before.blocks[range.before_start..range.before_end]
                    .iter()
                    .map(|b| b.id.clone())
                    .collect(),
                after_block_ids: after.blocks[range.after_start..range.after_end]
                    .iter()
                    .map(|b| b.id.clone())
                    .collect(),
            })
        })
        .collect()
}
pub(super) fn resolve(
    before: &mut NoteDocument,
    after: &mut NoteDocument,
    id: Option<&str>,
    keep: bool,
) -> Result<(), String> {
    if id.is_none() {
        if keep {
            *before = after.clone();
        } else {
            *after = before.clone();
        }
        return Ok(());
    }
    let chunk = chunks(before, after)?
        .into_iter()
        .find(|c| Some(c.id.as_str()) == id)
        .ok_or("review chunk changed; read the latest review")?;
    if keep {
        before.blocks.splice(
            chunk.before_index..chunk.before_index + chunk.before_block_ids.len(),
            after.blocks[chunk.after_index..chunk.after_index + chunk.after_block_ids.len()]
                .iter()
                .cloned(),
        );
    } else {
        after.blocks.splice(
            chunk.after_index..chunk.after_index + chunk.after_block_ids.len(),
            before.blocks[chunk.before_index..chunk.before_index + chunk.before_block_ids.len()]
                .iter()
                .cloned(),
        );
    }
    Ok(())
}
/// Ordinary editing updates B only outside unresolved review roots. Within a
/// changed root the old B is retained, including nested children and metadata.
pub(super) fn synchronize(
    before: &NoteDocument,
    current: &NoteDocument,
    edited: &NoteDocument,
) -> Result<NoteDocument, String> {
    let pending = chunks(before, current)?
        .into_iter()
        .flat_map(|c| c.before_block_ids.into_iter().chain(c.after_block_ids))
        .collect::<BTreeSet<_>>();
    let edited_by_id = edited
        .blocks
        .iter()
        .map(|b| (b.id.as_str(), b))
        .collect::<BTreeMap<_, _>>();
    let current_ids = current
        .blocks
        .iter()
        .map(|b| b.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut baseline = before.clone();
    baseline.blocks.retain(|b| {
        pending.contains(&b.id)
            || !current_ids.contains(b.id.as_str())
            || edited_by_id.contains_key(b.id.as_str())
    });
    for block in &mut baseline.blocks {
        if !pending.contains(&block.id)
            && let Some(edited) = edited_by_id.get(block.id.as_str())
        {
            *block = (*edited).clone();
        }
    }
    // Position non-pending roots against the next stable ID. Pending roots keep
    // their B values, while adjacent manual edits are not swallowed by a diff range.
    for (index, block) in edited.blocks.iter().enumerate().rev() {
        if pending.contains(&block.id) {
            continue;
        }
        if let Some(old) = baseline.blocks.iter().position(|b| b.id == block.id) {
            baseline.blocks.remove(old);
        }
        let position = edited.blocks[index + 1..]
            .iter()
            .find_map(|next| baseline.blocks.iter().position(|b| b.id == next.id))
            .unwrap_or(baseline.blocks.len());
        baseline.blocks.insert(position, block.clone());
    }
    Ok(baseline)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(parts: &[(&str, &str)]) -> NoteDocument {
        NoteDocument { title: None, schema_version: 1, page_id: "page".into(), blocks: parts.iter().map(|(id, text)| NoteDocumentBlock {
            id: (*id).into(), block_type: "paragraph".into(), props: serde_json::json!({}),
            content: serde_json::json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]}), children: vec![],
        }).collect() }
    }
    #[test]
    fn moved_identity_joins_fragments_and_undo_keeps_unique_identities() {
        let mut before = document(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let mut after = document(&[("b", "B"), ("a", "A"), ("c", "C")]);
        let shown = chunks(&before, &after).unwrap();
        assert_eq!(shown.len(), 1);
        resolve(&mut before, &mut after, Some(&shown[0].id), false).unwrap();
        assert!(equal(&before, &after));
        assert_eq!(after.blocks[0].id, "a");
    }
    #[test]
    fn nested_change_is_one_root_chunk_and_delete_is_anchored_at_real_position() {
        let before = document(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let mut after = before.clone();
        after.blocks.remove(1);
        let deletion = chunks(&before, &after).unwrap();
        assert_eq!(deletion[0].after_index, 1);
        assert!(deletion[0].after_block_ids.is_empty());
        let mut after = before.clone();
        after.blocks[0]
            .children
            .push(document(&[("child", "nested")]).blocks.remove(0));
        let nested = chunks(&before, &after).unwrap();
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].after_block_ids, ["a"]);
    }
    #[test]
    fn manual_insert_and_delete_outside_pending_roots_update_baseline_with_hidden_attrs() {
        let before = document(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let mut current = before.clone();
        current.blocks[0].content["items"][0]["text"] = serde_json::json!("AI A");
        let mut edited = current.clone();
        edited.blocks.remove(2);
        edited
            .blocks
            .insert(2, document(&[("new", "user")]).blocks.remove(0));
        let synced = synchronize(&before, &current, &edited).unwrap();
        assert_eq!(synced.blocks[0].content["items"][0]["text"], "A");
        assert_eq!(synced.blocks[2].id, "new");
        assert_eq!(chunks(&synced, &edited).unwrap().len(), 1);
    }
}
