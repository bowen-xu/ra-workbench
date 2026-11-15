//! Preserve sidecars on unchanged inline nodes and a uniquely corresponding edit.
use super::{Result, render};
use serde_json::{Value, json};
fn visible(item: &Value) -> Result<String> {
    render::inline(&json!([item]))
}
pub(crate) fn content(before: &Value, after: &mut Value) -> Result<()> {
    if before["kind"] == after["kind"] {
        let mut merged = before.clone();
        let object = merged.as_object_mut().ok_or("invalid content mapping")?;
        for key in [
            "items",
            "rows",
            "source",
            "assetId",
            "displayName",
            "mimeType",
            "caption",
            "targetKind",
            "targetId",
            "labelSnapshot",
            "quoteSnapshot",
        ] {
            object.remove(key);
        }
        for (key, value) in after.as_object().ok_or("invalid candidate content")? {
            object.insert(key.clone(), value.clone());
        }
        *after = merged;
    }
    if before["kind"] == "inline" && after["kind"] == "inline" {
        let old = before["items"].as_array().ok_or("invalid inline mapping")?;
        let next = after["items"]
            .as_array_mut()
            .ok_or("invalid inline candidate")?;
        if old.len().max(next.len()) > 2048 {
            return Err("inline mapping exceeds 2048 nodes; narrow the patch".into());
        }
        let mut used = std::collections::BTreeSet::new();
        let mut mapped = vec![None; next.len()];
        for (index, item) in next.iter().enumerate() {
            let text = visible(item)?;
            let matches = old
                .iter()
                .enumerate()
                .filter(|(position, _)| !used.contains(position))
                .map(|(position, item)| Ok((position, visible(item)?)))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|(_, value)| value == &text)
                .map(|(position, _)| position)
                .collect::<Vec<_>>();
            let position = if matches.contains(&index) && old.len() == next.len() {
                Some(index)
            } else if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            };
            if let Some(position) = position {
                mapped[index] = Some(position);
                used.insert(position);
            }
        }
        let next_len = next.len();
        let mut remaining = old
            .iter()
            .enumerate()
            .filter(|(index, _)| !used.contains(index))
            .map(|(index, _)| index);
        for (index, item) in next.iter_mut().enumerate() {
            if let Some(position) = mapped[index] {
                *item = old[position].clone();
            } else if let Some(position) = remaining.next()
                && old[position]["type"] == item["type"]
            {
                let known = [
                    "type",
                    "text",
                    "styles",
                    "textColor",
                    "backgroundColor",
                    "href",
                    "content",
                    "source",
                    "targetKind",
                    "targetId",
                    "labelSnapshot",
                ];
                if old.len() != next_len
                    && old[position]
                        .as_object()
                        .is_some_and(|value| value.keys().any(|key| !known.contains(&key.as_str())))
                {
                    return Err("inline metadata mapping is ambiguous; narrow the patch".into());
                }
                if old[position]["content"].is_array() && item["content"].is_array() {
                    let mut nested = json!({"kind":"inline","items":item["content"]});
                    content(
                        &json!({"kind":"inline","items":old[position]["content"]}),
                        &mut nested,
                    )?;
                    item["content"] = nested["items"].take();
                }
                let mut merged = old[position].clone();
                for key in [
                    "text",
                    "styles",
                    "textColor",
                    "backgroundColor",
                    "href",
                    "content",
                    "source",
                    "targetKind",
                    "targetId",
                    "labelSnapshot",
                ] {
                    merged
                        .as_object_mut()
                        .ok_or("invalid old inline node")?
                        .remove(key);
                }
                for (key, value) in item.as_object().ok_or("invalid new inline node")? {
                    merged[key] = value.clone();
                }
                *item = merged;
            }
        }
    } else if before["kind"] == "table" && after["kind"] == "table" {
        let old = before["rows"].as_array().ok_or("invalid old table")?;
        let next = after["rows"].as_array_mut().ok_or("invalid new table")?;
        if old.len() == next.len() {
            for (old, next) in old.iter().zip(next) {
                let old = old.as_array().ok_or("invalid row")?;
                let next = next.as_array_mut().ok_or("invalid row")?;
                if old.len() == next.len() {
                    for (old, next) in old.iter().zip(next) {
                        content(old, next)?;
                    }
                }
            }
        }
    }
    Ok(())
}
