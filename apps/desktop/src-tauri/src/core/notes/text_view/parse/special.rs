use super::super::xml::Part;
use super::*;

pub(super) fn apply(
    element: &Element,
    blocks: &mut Vec<NoteDocumentBlock>,
    depth: usize,
) -> Result<bool> {
    let props = json!({});
    let new = match element.name.as_str() {
        "toggle" => block(
            "toggleListItem",
            container(inline::markdown(&xml::inner(element), depth + 1)?),
            props,
        ),
        "children" => {
            blocks
                .last_mut()
                .ok_or("children must follow a parent block")?
                .children = parse_blocks(&element.text()?, depth + 1)?;
            return Ok(true);
        }
        "block-format" => {
            let parent = blocks
                .last_mut()
                .ok_or("block-format must follow a block")?;
            if element.attrs.contains_key("toggle") {
                if parent.block_type != "heading"
                    || !["true", "false"].contains(&element.attr("toggle"))
                {
                    return Err("toggle format requires a heading and true/false".into());
                }
                parent.props["isToggleable"] = json!(element.attr("toggle") == "true");
            }
            for (attribute, key) in [
                ("align", "textAlignment"),
                ("color", "textColor"),
                ("background", "backgroundColor"),
            ] {
                if !element.attr(attribute).is_empty() {
                    parent.props[key] = json!(element.attr(attribute));
                }
            }
            if !element.content.is_empty() {
                return Err("block-format must be empty".into());
            }
            return Ok(true);
        }
        "image" | "file" => {
            if !element.content.is_empty() {
                return Err("asset tags must be empty".into());
            }
            if let Some(handle) = element.attr("src").strip_prefix("opaque://") {
                if element.name != "file"
                    || element
                        .attrs
                        .keys()
                        .any(|key| key != "src" && key != "name")
                {
                    return Err("invalid opaque placeholder".into());
                }
                block(
                    "unsupported",
                    json!({"kind":"unsupported","originalType":element.attr("name"),"rawJson":""}),
                    json!({"opaqueHandle":handle}),
                )
            } else {
                let id = if element.attr("src").starts_with("import://") {
                    element.attr("src")
                } else {
                    element
                        .attr("src")
                        .strip_prefix("asset://")
                        .ok_or("assets require asset:// or authorized import:// handles")?
                };
                if id.is_empty() {
                    return Err("empty asset handle".into());
                }
                block(
                    element.name.as_str(),
                    json!({"kind":"asset","assetId":id,"displayName":element.attr("name"),"mimeType":element.attr("mime"),"caption":element.attr("caption")}),
                    props,
                )
            }
        }
        "child-page" => {
            if !element.content.is_empty() {
                return Err("child-page must be empty".into());
            }
            let target = super::super::reference_id(
                if element.name == "child-page" {
                    "note_page"
                } else {
                    element.attr("kind")
                },
                element.attr("target"),
            )?;
            block(
                "childPage",
                json!({"kind":"reference","targetKind":"note_page","targetId":target,"labelSnapshot":element.attr("label"),"quoteSnapshot":""}),
                props,
            )
        }
        "reference" if !element.attr("display").is_empty() => {
            let kind = element.attr("kind");
            let id = super::super::reference_id(element.attr("kind"), element.attr("target"))?;
            let block_type = match element.attr("display") {
                "block" => "researchObjectReference",
                "reading-note" => "readingNoteReference",
                _ => return Err("invalid reference display".into()),
            };
            block(
                block_type,
                json!({"kind":"reference","targetKind":kind,"targetId":id,"labelSnapshot":element.attr("label"),"quoteSnapshot":element.text()?}),
                props,
            )
        }
        "table" => {
            let mut rows = vec![];
            for row in &element.content {
                let Part::Element(row) = row else {
                    if let Part::Text(text) = row
                        && text.trim().is_empty()
                    {
                        continue;
                    }
                    return Err("table may contain only tr".into());
                };
                if row.name != "tr" {
                    return Err("table may contain only tr".into());
                }
                let mut cells = vec![];
                for cell in &row.content {
                    let Part::Element(cell) = cell else {
                        if let Part::Text(text) = cell
                            && text.trim().is_empty()
                        {
                            continue;
                        }
                        return Err("tr may contain only td".into());
                    };
                    if cell.name != "td" {
                        return Err("tr may contain only td".into());
                    }
                    cells.push(container(inline::markdown(&xml::inner(cell), depth + 1)?));
                }
                rows.push(cells);
            }
            block("table", json!({"kind":"table","rows":rows}), props)
        }
        "reference" | "u" | "span" | "strong" | "em" | "s" => return Ok(false),
        _ => return Err(format!("invalid block XML: {}", element.name)),
    };
    blocks.push(new);
    Ok(true)
}
