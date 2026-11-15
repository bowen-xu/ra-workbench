//! Stable entrance deltas; whole source documents are never restored over edits.
use super::{
    document_repository,
    error::NotesError,
    model::{NoteDocument, NoteDocumentBlock, NoteDocumentSaveRequest},
};
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Removal {
    block: NoteDocumentBlock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    children: Option<Vec<ChildIdentity>>,
    index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChildIdentity {
    id: String,
    children: Vec<ChildIdentity>,
}
fn child_identities(blocks: &[NoteDocumentBlock]) -> Vec<ChildIdentity> {
    blocks
        .iter()
        .map(|block| ChildIdentity {
            id: block.id.clone(),
            children: child_identities(&block.children),
        })
        .collect()
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Entries {
    pub page_id: String,
    pub saved: Vec<Removal>,
    pub baseline: Vec<Removal>,
}
fn capture(
    blocks: &[NoteDocumentBlock],
    parent: Option<&str>,
    targets: &HashSet<String>,
    proofs: &BTreeMap<String, String>,
    result: &mut Vec<Removal>,
) {
    for (index, block) in blocks.iter().enumerate() {
        if block.block_type != "childPage"
            && block.content["kind"] == "reference"
            && block.content["targetKind"] == "note_page"
            && block.content["targetId"].as_str().is_some_and(|id| {
                targets.contains(id)
                    && (block.props["pageEntry"] == true
                        || proofs.get(&block.id).is_some_and(|v| v == id))
            })
        {
            let mut entrance = block.clone();
            entrance.children.clear();
            result.push(Removal {
                block: entrance,
                children: Some(child_identities(&block.children)),
                index,
                parent_id: parent.map(str::to_owned),
                previous_id: index.checked_sub(1).map(|i| blocks[i].id.clone()),
                next_id: blocks.get(index + 1).map(|v| v.id.clone()),
            });
        }
        capture(&block.children, Some(&block.id), targets, proofs, result);
    }
}
pub(super) fn capture_sources(
    tx: &Transaction<'_>,
    targets: &HashSet<String>,
) -> Result<Vec<Entries>, NotesError> {
    let mut result = Vec::new();
    let has_baseline: bool = tx.query_row(
        "select exists(select 1 from sqlite_master where name='hbs_note_baselines')",
        [],
        |r| r.get(0),
    )?;
    for (id, proofs) in super::page_entries::proofs(tx, targets)? {
        let exists: bool = tx.query_row(
            "select exists(select 1 from note_pages where id=?1)",
            [&id],
            |r| r.get(0),
        )?;
        if !exists {
            continue;
        }
        let document = document_repository::load_document_value(tx, &id)?;
        let mut saved = Vec::new();
        capture(&document.blocks, None, targets, &proofs, &mut saved);
        if saved.is_empty() {
            continue;
        }
        let raw: Option<String> = if has_baseline {
            tx.query_row(
                "select baseline_json from hbs_note_baselines where page_id=?1",
                [&id],
                |r| r.get(0),
            )
            .optional()?
        } else {
            None
        };
        let mut baseline = saved.clone();
        if let Some(raw) = raw {
            baseline.clear();
            let document: NoteDocument = serde_json::from_str(&raw)
                .map_err(|e| NotesError::InvalidInput(format!("note_history_invalid: {e}")))?;
            capture(&document.blocks, None, targets, &proofs, &mut baseline);
        }
        result.push(Entries {
            page_id: id,
            saved,
            baseline,
        });
    }
    Ok(result)
}
fn promoted_ids(blocks: &[ChildIdentity], removed: &HashSet<String>) -> HashSet<String> {
    blocks
        .iter()
        .flat_map(|block| {
            if removed.contains(&block.id) {
                promoted_ids(&block.children, removed)
            } else {
                HashSet::from([block.id.clone()])
            }
        })
        .collect()
}
fn insert(
    blocks: &mut Vec<NoteDocumentBlock>,
    parent: Option<&str>,
    removal: &Removal,
    removed: &HashSet<String>,
) -> Result<bool, NotesError> {
    if parent != removal.parent_id.as_deref() {
        for block in blocks {
            if insert(&mut block.children, Some(&block.id), removal, removed)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if blocks.iter().any(|v| v.id == removal.block.id) {
        return Err(NotesError::InvalidInput(
            "note_entry_history_changed: 页面入口已存在".into(),
        ));
    }
    let fallback = child_identities(&removal.block.children);
    let children = promoted_ids(removal.children.as_deref().unwrap_or(&fallback), removed);
    let mut restored = removal.block.clone();
    restored.children = blocks
        .iter()
        .filter(|v| children.contains(&v.id))
        .cloned()
        .collect();
    blocks.retain(|v| !children.contains(&v.id));
    let at = blocks
        .iter()
        .position(|v| Some(&v.id) == removal.next_id.as_ref())
        .or_else(|| {
            blocks
                .iter()
                .position(|v| Some(&v.id) == removal.previous_id.as_ref())
                .map(|i| i + 1)
        })
        .unwrap_or(removal.index.min(blocks.len()));
    blocks.insert(at, restored);
    Ok(true)
}
fn restore(mut document: NoteDocument, entries: &[Removal]) -> Result<NoteDocument, NotesError> {
    let removed = entries.iter().map(|entry| entry.block.id.clone()).collect();
    for entry in entries {
        if !insert(&mut document.blocks, None, entry, &removed)? {
            return Err(NotesError::InvalidInput(
                "note_entry_history_changed: 页面入口原位置已不存在".into(),
            ));
        }
    }
    Ok(document)
}
pub(super) fn restore_sources(
    tx: &Transaction<'_>,
    entries: &[Entries],
) -> Result<Vec<String>, NotesError> {
    let mut result = Vec::new();
    for entry in entries {
        let before = document_repository::load_document_from(tx, &entry.page_id)?;
        let document = restore(before.document.clone(), &entry.saved)?;
        document_repository::save_preserving_in(
            tx,
            NoteDocumentSaveRequest {
                document,
                expected_revision: before.page.revision,
                save_reason: "structure".into(),
            },
            &before.document,
        )?;
        let has: bool = tx.query_row(
            "select exists(select 1 from sqlite_master where name='hbs_note_baselines')",
            [],
            |r| r.get(0),
        )?;
        if has {
            let raw: Option<String> = tx
                .query_row(
                    "select baseline_json from hbs_note_baselines where page_id=?1",
                    [&entry.page_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(raw) = raw {
                let document: NoteDocument = serde_json::from_str(&raw)
                    .map_err(|e| NotesError::InvalidInput(format!("note_history_invalid: {e}")))?;
                super::hbs::store_baseline(
                    tx,
                    &entry.page_id,
                    &restore(document, &entry.baseline)?,
                )
                .map_err(|e| NotesError::InvalidInput(format!("note_history_invalid: {e}")))?;
            }
        }
        result.push(entry.page_id.clone());
    }
    Ok(result)
}
