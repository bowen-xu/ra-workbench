//! Versioned authoring view; the note document remains the sole saved representation.
#[path = "inline_mapping.rs"]
pub(crate) mod inline_mapping;
#[path = "parse.rs"]
mod parse;
#[path = "protocol.rs"]
mod protocol;
#[path = "ranges.rs"]
mod ranges;
#[path = "reconcile.rs"]
mod reconcile;
#[path = "render.rs"]
mod render;
#[path = "xml.rs"]
mod xml;
pub(crate) use parse::parse;
pub(crate) use ranges::candidate as mapped_candidate;
pub(crate) use reconcile::candidate;
pub(crate) use render::{render, render_block};

pub(crate) fn version() -> u32 {
    protocol::DEFINITION.version
}
pub(crate) const INSTRUCTIONS: &str = include_str!("syntax.md");
type Result<T> = std::result::Result<T, String>;
fn merge_text_items(items: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut result: Vec<serde_json::Value> = vec![];
    for item in items {
        // Empty editor runs carry no visible text; retain their metadata through
        // unchanged-node reconciliation, not empty Markdown style delimiters.
        if string(&item, "type") == "text" && string(&item, "text").is_empty() {
            continue;
        }
        if string(&item, "type") == "text"
            && let Some(last) = result.last_mut()
            && string(last, "type") == "text"
        {
            let mut left = last.clone();
            let mut right = item.clone();
            left.as_object_mut().unwrap().remove("text");
            right.as_object_mut().unwrap().remove("text");
            if left == right {
                last["text"] =
                    serde_json::json!(format!("{}{}", string(last, "text"), string(&item, "text")));
                continue;
            }
        }
        result.push(item);
    }
    result
}
fn string<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn reference_id(kind: &str, target: &str) -> Result<String> {
    let path = research_agent_core::text_patch::address::parse(target)?;
    let id = path.last().ok_or("找不到文件对象")?;
    if research_agent_core::text_patch::address::parse(
        &research_agent_core::text_patch::address::reference(kind, id),
    )? != path
    {
        return Err("object reference does not match its resource directory".into());
    }
    Ok(id.clone())
}

fn validate_view(document: &crate::notes::NoteDocument) -> Result<()> {
    validate_candidate(document, document)
}
fn validate_candidate(
    document: &crate::notes::NoteDocument,
    before: &crate::notes::NoteDocument,
) -> Result<()> {
    let mut document = document.clone();
    fn visit(blocks: &mut [crate::notes::NoteDocumentBlock]) -> Result<()> {
        for block in blocks {
            if block.content["kind"] == "asset"
                && let Some(id) = block.content["assetId"]
                    .as_str()
                    .and_then(|value| value.strip_prefix("import://"))
            {
                if id.is_empty() || id.len() > 100 || id.contains(['/', '\0']) {
                    return Err("invalid import handle".into());
                }
                // Import handles are resolved before persistence, never admitted
                // to the authoritative document validator or save path.
                block.content["assetId"] = serde_json::json!("0".repeat(64));
            }
            visit(&mut block.children)?;
        }
        Ok(())
    }
    visit(&mut document.blocks)?;
    crate::notes::document_validation::validate_preserving(&document, before)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
