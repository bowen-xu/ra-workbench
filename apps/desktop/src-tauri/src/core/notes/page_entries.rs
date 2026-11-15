//! Explicit page-entry lifecycle edits. Ordinary references are never inferred
//! from title, nesting or target alone. S and its scoped B edit share the tx.
use super::{
    document_repository,
    error::NotesError,
    model::{NoteDocument, NoteDocumentBlock, NoteDocumentSaveRequest},
};
use rusqlite::{OptionalExtension, Transaction};
use std::collections::{BTreeMap, HashSet};
type Proofs = BTreeMap<String, BTreeMap<String, String>>;

fn target(block: &NoteDocumentBlock) -> Option<&str> {
    (block.content["kind"] == "reference" && block.content["targetKind"] == "note_page")
        .then(|| block.content["targetId"].as_str())
        .flatten()
}
fn collect(blocks: &[NoteDocumentBlock], id: &str, entries: &mut BTreeMap<String, String>) {
    for block in blocks {
        if target(block) == Some(id) {
            entries.insert(block.id.clone(), id.to_owned());
        }
        collect(&block.children, id, entries);
    }
}
pub(super) fn proofs(
    tx: &Transaction<'_>,
    targets: &HashSet<String>,
) -> Result<Proofs, NotesError> {
    let mut result = Proofs::new();
    for id in targets {
        let mut query = tx.prepare("select b.page_id,b.id from note_links l join note_blocks b on b.id=l.source_block_id where l.target_kind='note_page' and l.target_id=?1 and b.status='active' and json_extract(b.props_json,'$.pageEntry')=1")?;
        for row in query.query_map([id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (page, block) = row?;
            result.entry(page).or_default().insert(block, id.clone());
        }
        let has_receipts: bool=tx.query_row("select exists(select 1 from sqlite_master where type='table' and name='hbs_commit_receipts')",[],|row|row.get(0))?;
        if !has_receipts {
            continue;
        }
        let mut query=tx.prepare("select result_json from hbs_commit_receipts where kind='note' and resource_id=?1 and json_extract(result_json,'$.creationFingerprint') is not null")?;
        for row in query.query_map([id], |row| row.get::<_, String>(0))? {
            let receipt: serde_json::Value = serde_json::from_str(&row?).map_err(|e| {
                NotesError::InvalidInput(format!("note_entry_receipt_invalid: {e}"))
            })?;
            if receipt["source"].is_null() {
                continue;
            }
            let document: NoteDocument =
                serde_json::from_value(receipt["source"]["document"].clone()).map_err(|e| {
                    NotesError::InvalidInput(format!("note_entry_receipt_invalid: {e}"))
                })?;
            collect(
                &document.blocks,
                id,
                result.entry(document.page_id.clone()).or_default(),
            );
        }
    }
    Ok(result)
}
fn remove(
    blocks: Vec<NoteDocumentBlock>,
    targets: &HashSet<String>,
    proofs: &BTreeMap<String, String>,
) -> Vec<NoteDocumentBlock> {
    blocks
        .into_iter()
        .flat_map(|mut block| {
            let owned = target(&block).is_some_and(|id| {
                targets.contains(id)
                    && (block.block_type == "childPage"
                        || block.props["pageEntry"] == true
                        || proofs.get(&block.id).is_some_and(|proof| proof == id))
            });
            block.children = remove(block.children, targets, proofs);
            if owned { block.children } else { vec![block] }
        })
        .collect()
}
pub(crate) fn detach(
    tx: &Transaction<'_>,
    trashed: &HashSet<String>,
) -> Result<HashSet<String>, NotesError> {
    let mut changed = HashSet::new();
    for (id, proof) in proofs(tx, trashed)? {
        // A source may have been permanently removed since the old receipt.
        let exists: bool = tx.query_row(
            "select exists(select 1 from note_pages where id=?1)",
            [&id],
            |row| row.get(0),
        )?;
        if !exists {
            continue;
        }
        let before = document_repository::load_document_from(tx, &id)?;
        let mut after = before.document.clone();
        after.blocks = remove(after.blocks, trashed, &proof);
        if serde_json::to_value(&after).ok() == serde_json::to_value(&before.document).ok() {
            continue;
        }
        document_repository::save_preserving_in(
            tx,
            NoteDocumentSaveRequest {
                document: after,
                expected_revision: before.page.revision,
                save_reason: "structure".into(),
            },
            &before.document,
        )?;
        changed.insert(id);
    }
    Ok(changed)
}
pub(crate) fn synchronize_baselines(
    tx: &Transaction<'_>,
    changed: &HashSet<String>,
    targets: &HashSet<String>,
    trashed: &HashSet<String>,
) -> Result<(), NotesError> {
    let exists:bool=tx.query_row("select exists(select 1 from sqlite_master where type='table' and name='hbs_note_baselines')",[],|row|row.get(0))?;
    if !exists {
        return Ok(());
    }
    let proofs = proofs(tx, trashed)?;
    for id in changed {
        let raw: Option<String> = tx
            .query_row(
                "select baseline_json from hbs_note_baselines where page_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            continue;
        };
        let mut baseline: NoteDocument = serde_json::from_str(&raw)
            .map_err(|e| NotesError::InvalidInput(format!("note_entry_baseline_invalid: {e}")))?;
        let current = document_repository::load_document_value(tx, id)?;
        let owned = |block: &NoteDocumentBlock| {
            block.block_type == "childPage"
                && target(block).is_some_and(|target| targets.contains(target))
        };
        baseline.blocks.retain(|block| !owned(block));
        for (index, block) in current
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| owned(block))
        {
            let next = current.blocks[index + 1..]
                .iter()
                .find_map(|next| baseline.blocks.iter().position(|item| item.id == next.id));
            let previous = current.blocks[..index].iter().rev().find_map(|previous| {
                baseline
                    .blocks
                    .iter()
                    .position(|item| item.id == previous.id)
                    .map(|at| at + 1)
            });
            let at = next
                .or(previous)
                .unwrap_or(index.min(baseline.blocks.len()));
            baseline.blocks.insert(at, block.clone());
        }
        baseline.blocks = remove(
            baseline.blocks,
            trashed,
            proofs.get(id).unwrap_or(&BTreeMap::new()),
        );
        super::hbs::store_baseline(tx, id, &baseline)
            .map_err(|e| NotesError::InvalidInput(format!("note_entry_baseline_failed: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "page_entries/tests.rs"]
mod tests;
