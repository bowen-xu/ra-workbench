//! Editor projections may omit application sidecars and materialize schema defaults.
use crate::notes::{NoteDocument, NoteDocumentBlock};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn preserve(
    before: &NoteDocument,
    mut edited: NoteDocument,
) -> Result<NoteDocument, String> {
    if edited.page_id != before.page_id || edited.schema_version != before.schema_version {
        return Err("editor document identity or schema changed".into());
    }
    fn index<'a>(
        blocks: &'a [NoteDocumentBlock],
        found: &mut BTreeMap<&'a str, &'a NoteDocumentBlock>,
    ) {
        for block in blocks {
            found.insert(&block.id, block);
            index(&block.children, found);
        }
    }
    fn overlay(
        blocks: &mut [NoteDocumentBlock],
        found: &BTreeMap<&str, &NoteDocumentBlock>,
    ) -> Result<(), String> {
        for block in blocks {
            if let Some(prior) = found.get(block.id.as_str()) {
                if !crate::notes::document_validation::known_block(&prior.block_type) {
                    if serde_json::to_value(&*block).map_err(|e| e.to_string())?
                        != serde_json::to_value(*prior).map_err(|e| e.to_string())?
                    {
                        return Err(
                            "future blocks must remain verbatim; preserve editor input".into()
                        );
                    }
                } else {
                    let mut props = prior
                        .props
                        .as_object()
                        .ok_or("invalid prior block props")?
                        .clone();
                    if prior.block_type != block.block_type {
                        for key in ["level", "checked", "language"] {
                            props.remove(key);
                        }
                    }
                    for (key, value) in block
                        .props
                        .as_object()
                        .ok_or("invalid editor block props")?
                    {
                        let default = match key.as_str() {
                            "textAlignment" => Some("left"),
                            "textColor" | "backgroundColor" => Some("default"),
                            _ => None,
                        };
                        if prior.props.get(key).is_none()
                            && default.is_some_and(|default| value == default)
                        {
                            continue;
                        }
                        props.insert(key.clone(), value.clone());
                    }
                    block.props = Value::Object(props);
                    if block.block_type == prior.block_type {
                        crate::notes::text_view::inline_mapping::content(
                            &prior.content,
                            &mut block.content,
                        )?;
                    }
                }
            }
            overlay(&mut block.children, found)?;
        }
        Ok(())
    }
    let mut found = BTreeMap::new();
    index(&before.blocks, &mut found);
    overlay(&mut edited.blocks, &found)?;
    crate::notes::document_validation::validate_preserving(&edited, before)?;
    Ok(edited)
}
