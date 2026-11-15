use super::{Result, string};
use crate::notes::{NoteDocument, NoteDocumentBlock};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn markdown(value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        // A literal backslash followed by an opening bracket must not become
        // a formula when this authoring view is read back through the codec.
        if character == '\\' && matches!(chars.peek(), Some('(' | '[')) {
            result.push_str("\\\\");
            result.push_str(if chars.next() == Some('(') {
                "&#40;"
            } else {
                "&#91;"
            });
        } else {
            if matches!(character, '\n' | '\r' | '\t') {
                result.push_str(&format!("&#{};", character as u32));
                continue;
            }
            if "\\`*_[]<>#$~|-.()+!=&".contains(character) {
                result.push('\\');
            }
            result.push(character);
        }
    }
    result
}
fn cdata(value: &str) -> String {
    format!("<![CDATA[{}]]>", value.replace("]]>", "]]]]><![CDATA[>"))
}

pub(crate) fn inline(items: &Value) -> Result<String> {
    let mut result = String::new();
    // Match the parser's adjacent-text coalescing before choosing delimiters.
    // Editor node splitting must not change the canonical authoring view.
    let items = super::merge_text_items(items.as_array().ok_or("invalid inline items")?.clone());
    for item in &items {
        let code = string(item, "type") == "text"
            && item["styles"]
                .as_array()
                .is_some_and(|styles| styles.iter().any(|style| style == "code"));
        let mut text = match string(item, "type") {
            "text" if code => {
                let raw = string(item, "text");
                if raw.contains('\n') {
                    return Err("multiline inline code cannot be projected losslessly".into());
                }
                let fence = "`".repeat(longest_run(raw, '`') + 1);
                // CommonMark strips padding only for non-space-only code spans.
                if raw.chars().all(|value| value == ' ') {
                    format!("{fence}{raw}{fence}")
                } else {
                    format!("{fence} {raw} {fence}")
                }
            }
            "text" => markdown(string(item, "text")),
            "link" => format!(
                "[{}](<{}>)",
                inline(&item["content"])?,
                escape(string(item, "href"))
            ),
            "inlineFormula" => format!("\\({}\\)", string(item, "source")),
            "mention" => format!(
                "<reference kind=\"{}\" target=\"{}\">{}</reference>",
                escape(string(item, "targetKind")),
                escape(&research_agent_core::text_patch::address::reference(
                    string(item, "targetKind"),
                    string(item, "targetId")
                )),
                escape(&if item["content"].is_array() {
                    inline(&item["content"])?
                } else {
                    markdown(string(item, "labelSnapshot"))
                })
            ),
            _ => return Err("unsupported inline type".into()),
        };
        if string(item, "type") == "text" {
            text = boundary_whitespace(&text);
            // CommonMark delimiter flanking depends on Unicode punctuation and
            // neighbouring text. Explicit tags cover unsafe edges without
            // inserting/removing actual characters or relaxing round-trip checks.
            let raw = string(item, "text");
            let styles = item["styles"].as_array().ok_or("missing styles")?;
            let kinds = ["bold", "italic", "strike", "underline", "code"];
            let explicit = kinds
                .iter()
                .filter(|style| styles.iter().any(|value| value == *style))
                .count()
                > 1
                || !raw.starts_with(char::is_alphanumeric)
                || !raw.ends_with(char::is_alphanumeric);
            if styles.iter().any(|style| {
                !style.as_str().is_some_and(|style| {
                    ["bold", "italic", "strike", "underline", "code"].contains(&style)
                })
            }) {
                return Err("unsupported style".into());
            }
            for style in kinds {
                if !styles.iter().any(|value| value == style) {
                    continue;
                }
                text = match style {
                    "bold" if explicit => format!("<strong>{}</strong>", escape(&text)),
                    "italic" if explicit => format!("<em>{}</em>", escape(&text)),
                    "strike" if explicit => format!("<s>{}</s>", escape(&text)),
                    "bold" => format!("**{text}**"),
                    "italic" => format!("*{text}*"),
                    "strike" => format!("~~{text}~~"),
                    "underline" => format!("<u>{}</u>", escape(&text)),
                    "code" => text,
                    _ => return Err("unsupported style".into()),
                };
            }
            if item.get("textColor").is_some() || item.get("backgroundColor").is_some() {
                let mut attributes = String::new();
                for (key, name) in [("textColor", "color"), ("backgroundColor", "background")] {
                    if let Some(color) = item[key].as_str() {
                        attributes.push_str(&format!(" {name}=\"{}\"", escape(color)));
                    }
                }
                text = format!("<span{attributes}>{}</span>", escape(&text));
            }
        }
        result.push_str(&text);
    }
    Ok(boundary_whitespace(&result))
}

// Literal edge whitespace is syntax to CommonMark. Entities remain text and
// preserve the exact code points, including whitespace-only paragraphs/labels.
fn boundary_whitespace(value: &str) -> String {
    let start = value.len() - value.trim_start_matches(char::is_whitespace).len();
    let end = value.trim_end_matches(char::is_whitespace).len();
    let mut output = String::new();
    for (index, character) in value.char_indices() {
        if index < start || index >= end {
            output.push_str(&format!("&#{};", character as u32));
        } else {
            output.push(character);
        }
    }
    output
}

fn longest_run(text: &str, character: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for value in text.chars() {
        if value == character {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}
fn ordinary_list(block: &NoteDocumentBlock) -> bool {
    matches!(
        block.block_type.as_str(),
        "bulletListItem" | "numberedListItem" | "checkListItem"
    ) && !inline(&block.content["items"]).is_ok_and(|text| text.contains('\n'))
        && ["textColor", "backgroundColor"].iter().all(|key| {
            block.props[*key]
                .as_str()
                .is_none_or(|value| value == "default")
        })
        && block.props["textAlignment"]
            .as_str()
            .is_none_or(|value| value == "left")
        && block.children.iter().all(ordinary_list)
}

pub(crate) fn render_block(block: &NoteDocumentBlock) -> Result<String> {
    if !crate::notes::document_validation::known_block(&block.block_type) {
        let raw = serde_json::to_vec(block).map_err(|error| error.to_string())?;
        return Ok(format!(
            "<file src=\"opaque://{:x}\" name=\"{}\"/>",
            Sha256::digest(raw),
            escape(&block.block_type)
        ));
    }
    let content = &block.content;
    let mut text = match string(content, "kind") {
        "inline" => {
            let body = inline(&content["items"])?;
            match block.block_type.as_str() {
                "heading" => format!(
                    "{} {body}",
                    "#".repeat(block.props["level"].as_u64().unwrap_or(2) as usize)
                ),
                "toggleListItem" => format!("<toggle>{}</toggle>", cdata(&body)),
                "bulletListItem" => format!("- {body}"),
                "numberedListItem" => format!("1. {body}"),
                "checkListItem" => format!(
                    "- [{}] {body}",
                    if block.props["checked"] == true {
                        "x"
                    } else {
                        " "
                    }
                ),
                "quote" => body
                    .lines()
                    .map(|line| format!("> {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                "codeBlock" => {
                    let plain = content["items"]
                        .as_array()
                        .ok_or("code content")?
                        .iter()
                        .map(|item| string(item, "text"))
                        .collect::<String>();
                    let fence = "`".repeat(3.max(longest_run(&plain, '`') + 1));
                    format!(
                        "{fence}{}\n{plain}\n{fence}",
                        string(&block.props, "language")
                    )
                }
                _ => {
                    if body.is_empty() {
                        "<span></span>".into()
                    } else {
                        body
                    }
                }
            }
        }
        "none" => "---".into(),
        "formula" => format!("\\[\n{}\n\\]", string(content, "source")),
        "asset" => format!(
            "<{} src=\"asset://{}\" name=\"{}\" mime=\"{}\" caption=\"{}\"/>",
            if block.block_type == "image" {
                "image"
            } else {
                "file"
            },
            escape(string(content, "assetId")),
            escape(string(content, "displayName")),
            escape(string(content, "mimeType")),
            escape(string(content, "caption"))
        ),
        "reference" => {
            let kind = escape(string(content, "targetKind"));
            let target = escape(&research_agent_core::text_patch::address::reference(
                string(content, "targetKind"),
                string(content, "targetId"),
            ));
            let label = escape(string(content, "labelSnapshot"));
            if block.block_type == "childPage" {
                format!("<child-page target=\"{target}\" label=\"{label}\"/>")
            } else {
                format!(
                    "<reference kind=\"{kind}\" target=\"{target}\" display=\"{}\" label=\"{label}\">{}</reference>",
                    if block.block_type == "readingNoteReference" {
                        "reading-note"
                    } else {
                        "block"
                    },
                    cdata(string(content, "quoteSnapshot"))
                )
            }
        }
        "table" => {
            let rows = content["rows"].as_array().ok_or("invalid table")?;
            let cells = rows
                .iter()
                .map(|row| {
                    row.as_array()
                        .ok_or("invalid row")?
                        .iter()
                        .map(|cell| inline(&cell["items"]))
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()?;
            if let Some(first) = cells.first()
                && !first.is_empty()
                && cells.iter().all(|row| {
                    row.len() == first.len() && row.iter().all(|cell| !cell.contains('\n'))
                })
            {
                let line = |row: &[String]| format!("| {} |", row.join(" | "));
                let mut table = vec![line(first), line(&vec!["---".into(); first.len()])];
                table.extend(cells[1..].iter().map(|row| line(row)));
                table.join("\n")
            } else {
                let mut text = String::from("<table>");
                for row in content["rows"].as_array().ok_or("invalid table")? {
                    text.push_str("<tr>");
                    for cell in row.as_array().ok_or("invalid row")? {
                        text.push_str(&format!("<td>{}</td>", cdata(&inline(&cell["items"])?)));
                    }
                    text.push_str("</tr>");
                }
                text.push_str("</table>");
                text
            }
        }
        "unsupported" => {
            let handle = block.props["opaqueHandle"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(content).unwrap_or_default())
                    )
                });
            format!(
                "<file src=\"opaque://{handle}\" name=\"{}\"/>",
                escape(string(content, "originalType"))
            )
        }
        _ => return Err("unknown block content".into()),
    };
    let mut attributes = String::new();
    for (key, name) in [
        ("textAlignment", "align"),
        ("textColor", "color"),
        ("backgroundColor", "background"),
    ] {
        if let Some(value) = block.props[key].as_str()
            && value != "default"
            && !(key == "textAlignment" && value == "left")
        {
            attributes.push_str(&format!(" {name}=\"{}\"", escape(value)));
        }
    }
    if block.block_type == "heading" && block.props["isToggleable"] == true {
        attributes.push_str(" toggle=\"true\"");
    }
    if !attributes.is_empty() {
        text.push_str(&format!("\n\n<block-format{attributes}/>"));
    }
    if !block.children.is_empty() && ordinary_list(block) {
        let nested = render_blocks(&block.children)?
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        text.push_str(&format!("\n{nested}"));
    } else if !block.children.is_empty() {
        text.push_str(&format!(
            "\n\n<children>{}</children>",
            cdata(&render_blocks(&block.children)?)
        ));
    }
    Ok(text)
}

pub(super) fn render_blocks(blocks: &[NoteDocumentBlock]) -> Result<String> {
    blocks
        .iter()
        .map(render_block)
        .collect::<Result<Vec<_>>>()
        .map(|values| values.join("\n\n"))
}
pub(crate) fn render(document: &NoteDocument, title: &str) -> Result<String> {
    Ok(format!(
        "---\ntitle: {}\n---\n\n{}\n",
        serde_json::to_string(title).map_err(|_| "invalid title")?,
        render_blocks(&document.blocks)?
    ))
}
