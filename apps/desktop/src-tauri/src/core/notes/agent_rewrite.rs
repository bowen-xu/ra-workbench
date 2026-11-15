//! Pure previews over frozen note evidence. This module cannot save documents.
#[path = "agent_rewrite/candidate.rs"]
mod candidate;
use super::{
    document_validation::validate_document,
    model::{NoteDocument, NoteDocumentBlock},
};
pub(crate) use candidate::candidate;
use research_agent_core::submission::{ContextSourceKind, ContextText};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InlineRewrite {
    pub block_id: String,
    pub content: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlockRewritePreview {
    pub block_id: String,
    pub before: Value,
    pub after: Value,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrozenNote {
    schema_version: u8,
    page_id: Uuid,
    title: String,
    page_revision: i64,
    blocks: Vec<NoteDocumentBlock>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Anchor {
    schema_version: u8,
    page_id: Uuid,
    page_revision: i64,
    block_id: Option<String>,
}

/// Only existing inline blocks inside the frozen source may change. Block IDs,
/// type, props, children and reference identities are never supplied by a model.
/// Unknown fields are rejected at changed boundaries, never silently discarded.
pub(crate) fn preview(
    source: &ContextText,
    edits: &[InlineRewrite],
) -> Result<Vec<BlockRewritePreview>, &'static str> {
    let (frozen, _) = decode_source(source)?;
    preview_frozen(frozen, edits)
}

fn decode_source(source: &ContextText) -> Result<(FrozenNote, Anchor), &'static str> {
    if source.kind != ContextSourceKind::Note || source.text.len() > 8 * 1024 * 1024 {
        return Err("invalid source or edit budget");
    }
    let raw: Value = serde_json::from_str(&source.text).map_err(|_| "invalid source")?;
    let frozen: FrozenNote = serde_json::from_value(raw.clone()).map_err(|_| "invalid source")?;
    // NoteDocumentBlock's compatibility decoder ignores unknown block fields.
    // A proposal must not lose them when deriving its baseline.
    if serde_json::to_value(&frozen).map_err(|_| "invalid source")? != raw {
        return Err("unsupported source fields");
    }
    let raw_anchor: Value = serde_json::from_str(source.anchor.as_deref().ok_or("missing anchor")?)
        .map_err(|_| "invalid anchor")?;
    let anchor: Anchor =
        serde_json::from_value(raw_anchor.clone()).map_err(|_| "invalid anchor")?;
    if serde_json::to_value(&anchor).map_err(|_| "invalid anchor")? != raw_anchor {
        return Err("unsupported anchor fields");
    }
    let identity =
        serde_json::to_vec(&(anchor.page_id, &anchor.block_id)).map_err(|_| "invalid anchor")?;
    if frozen.schema_version != 1
        || anchor.schema_version != 1
        || frozen.page_id.is_nil()
        || frozen.page_revision < 0
        || frozen.page_revision > 9_007_199_254_740_991
        || source.resource_id != frozen.page_id.to_string()
        || anchor.page_id != frozen.page_id
        || anchor.page_revision != frozen.page_revision
        || source.source_id != format!("note:{:x}", Sha256::digest(identity))
        || source.revision
            != format!(
                "revision:{}:sha256:{:x}",
                frozen.page_revision,
                Sha256::digest(source.text.as_bytes())
            )
        || anchor
            .block_id
            .as_ref()
            .is_some_and(|id| frozen.blocks.len() != 1 || frozen.blocks[0].id != *id)
    {
        return Err("source identity mismatch");
    }
    Ok((frozen, anchor))
}

fn preview_frozen(
    frozen: FrozenNote,
    edits: &[InlineRewrite],
) -> Result<Vec<BlockRewritePreview>, &'static str> {
    if edits.is_empty() || edits.len() > 32 {
        return Err("invalid edit budget");
    }
    let mut document = NoteDocument {
        title: None,
        schema_version: 1,
        page_id: frozen.page_id.to_string(),
        blocks: frozen.blocks,
    };
    validate_document(&document).map_err(|_| "invalid source document")?;
    let mut seen = HashSet::new();
    let mut result = vec![];
    for edit in edits {
        if !seen.insert(&edit.block_id) {
            return Err("duplicate block edit");
        }
        let block = find_mut(&mut document.blocks, &edit.block_id).ok_or("block outside source")?;
        if !matches!(
            block.block_type.as_str(),
            "paragraph"
                | "heading"
                | "bulletListItem"
                | "numberedListItem"
                | "checkListItem"
                | "quote"
                | "codeBlock"
        ) {
            return Err("block is not rewritable inline text");
        }
        let before_references = references(&block.content)?;
        let after_references = references(&edit.content)?;
        if before_references != after_references {
            return Err("rewrite must preserve references");
        }
        if block.content == edit.content {
            return Err("rewrite has no change");
        }
        result.push(BlockRewritePreview {
            block_id: block.id.clone(),
            before: block.content.clone(),
            after: edit.content.clone(),
        });
        block.content = edit.content.clone();
    }
    validate_document(&document).map_err(|_| "invalid replacement document")?;
    Ok(result)
}

fn find_mut<'a>(
    blocks: &'a mut [NoteDocumentBlock],
    id: &str,
) -> Option<&'a mut NoteDocumentBlock> {
    blocks.iter_mut().find_map(|block| {
        if block.id == id {
            Some(block)
        } else {
            find_mut(&mut block.children, id)
        }
    })
}

fn keys(value: &Value, expected: &[&str]) -> Result<(), &'static str> {
    let object = value.as_object().ok_or("expected object")?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err("unknown or missing inline field");
    }
    Ok(())
}

fn optional_keys(value: &Value, required: &[&str], optional: &[&str]) -> Result<(), &'static str> {
    let object = value.as_object().ok_or("expected object")?;
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err("unknown or missing inline field");
    }
    Ok(())
}

/// Compare a multiset: reordering is allowed, silently losing a repeated citation
/// is not. Display text may be revised; target identity and occurrence survive.
fn references(content: &Value) -> Result<Vec<String>, &'static str> {
    keys(content, &["kind", "items"])?;
    if content["kind"] != "inline" {
        return Err("expected inline content");
    }
    let mut references = vec![];
    for item in content["items"].as_array().ok_or("expected inline items")? {
        match item["type"].as_str() {
            Some("text") => optional_keys(
                item,
                &["type", "text", "styles"],
                &["textColor", "backgroundColor"],
            )?,
            Some("inlineFormula") => keys(item, &["type", "source"])?,
            Some("link") => {
                keys(item, &["type", "href", "content"])?;
                for part in item["content"].as_array().ok_or("expected link text")? {
                    optional_keys(
                        part,
                        &["type", "text", "styles"],
                        &["textColor", "backgroundColor"],
                    )?;
                }
                references.push(serde_json::json!(["link", item["href"]]).to_string());
            }
            Some("mention") => {
                optional_keys(
                    item,
                    &["type", "targetKind", "targetId", "labelSnapshot"],
                    &["content"],
                )?;
                if let Some(content) = item.get("content") {
                    for part in content.as_array().ok_or("expected mention text")? {
                        optional_keys(
                            part,
                            &["type", "text", "styles"],
                            &["textColor", "backgroundColor"],
                        )?;
                    }
                }
                references.push(
                    serde_json::json!(["mention", item["targetKind"], item["targetId"]])
                        .to_string(),
                );
            }
            _ => return Err("unsupported inline item"),
        }
    }
    references.sort();
    Ok(references)
}

#[cfg(test)]
#[path = "agent_rewrite/tests.rs"]
mod tests;
