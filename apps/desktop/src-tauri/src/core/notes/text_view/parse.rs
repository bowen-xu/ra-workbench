use super::{
    Result, string,
    xml::{self, Element},
};
use crate::notes::{NoteDocument, NoteDocumentBlock};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[path = "parse/extensions.rs"]
mod extensions;
#[path = "parse/inline.rs"]
mod inline;
use extensions::{Extension, Extensions, preprocess};
#[path = "parse/special.rs"]
mod special;

pub(super) fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_MATH
}
pub(super) fn block(kind: &str, content: Value, props: Value) -> NoteDocumentBlock {
    NoteDocumentBlock {
        id: uuid::Uuid::new_v4().to_string(),
        block_type: kind.into(),
        props,
        content,
        children: vec![],
    }
}
pub(super) fn text(value: &str, styles: &[&str]) -> Value {
    json!({"type":"text","text":value,"styles":styles})
}
pub(super) fn container(items: Vec<Value>) -> Value {
    json!({"kind":"inline","items":items})
}

pub(crate) fn parse(page_id: &str, source: &str) -> Result<(String, NoteDocument)> {
    if source.len() > 8 * 1024 * 1024 {
        return Err("note view exceeds 8 MiB".into());
    }
    let rest = source
        .strip_prefix("---\n")
        .ok_or("missing note title frontmatter")?;
    let end = rest
        .find("\n---\n")
        .ok_or("invalid note title frontmatter")?;
    if end > 4096 {
        return Err("note title frontmatter exceeds 4 KiB".into());
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Header {
        title: String,
    }
    let header: Header = serde_saphyr::from_str(&rest[..end]).map_err(|error| error.to_string())?;
    if header.title.trim().is_empty() || header.title.chars().count() > 240 {
        return Err("invalid note title".into());
    }
    let blocks = parse_blocks(&rest[end + 5..], 0)?;
    let document = NoteDocument {
        title: None,
        schema_version: 1,
        page_id: page_id.into(),
        blocks,
    };
    super::validate_view(&document)?;
    Ok((header.title, document))
}

pub(super) fn parse_blocks(source: &str, depth: usize) -> Result<Vec<NoteDocumentBlock>> {
    if depth > 64 {
        return Err("note depth limit".into());
    }
    let (input, xmls) = preprocess(source)?;
    let events: Vec<_> = Parser::new_ext(&input, options()).collect();
    let mut result = vec![];
    let mut position = 0;
    while position < events.len() {
        let event = &events[position];
        match event {
            Event::Start(Tag::Paragraph) | Event::Start(Tag::Heading { .. }) => {
                let heading = if let Event::Start(Tag::Heading { level, .. }) = event {
                    Some(*level as u8)
                } else {
                    None
                };
                let end = if heading.is_some() {
                    TagEnd::Heading(match event {
                        Event::Start(Tag::Heading { level, .. }) => *level,
                        _ => unreachable!(),
                    })
                } else {
                    TagEnd::Paragraph
                };
                position += 1;
                let start = position;
                while position < events.len() && events[position] != Event::End(end) {
                    position += 1;
                }
                let parts = &events[start..position];
                if parts.len() == 1
                    && let Event::DisplayMath(formula) = &parts[0]
                {
                    result.push(block(
                        "formulaBlock",
                        json!({"kind":"formula","source":formula.trim()}),
                        json!({}),
                    ));
                    position += 1;
                    continue;
                }
                if parts.len() == 1
                    && let Event::Text(token) = &parts[0]
                    && let Some(extension) = xmls.get(token.as_ref())
                    && apply_extension(extension, &mut result, depth)?
                {
                    position += 1;
                    continue;
                }
                let items = inline::events(parts, &xmls, depth)?;
                result.push(block(
                    if heading.is_some() {
                        "heading"
                    } else {
                        "paragraph"
                    },
                    container(items),
                    heading.map_or(json!({}), |level| json!({"level":level})),
                ));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(name) => name.as_ref(),
                    CodeBlockKind::Indented => "",
                };
                let mut body = String::new();
                position += 1;
                while position < events.len() && events[position] != Event::End(TagEnd::CodeBlock) {
                    if let Event::Text(part) = &events[position] {
                        body.push_str(part);
                    }
                    position += 1;
                }
                if body.ends_with('\n') {
                    body.pop();
                }
                result.push(block(
                    "codeBlock",
                    container(vec![text(&body, &[])]),
                    json!({"language":language}),
                ));
            }
            Event::DisplayMath(formula) => result.push(block(
                "formulaBlock",
                json!({"kind":"formula","source":formula.as_ref()}),
                json!({}),
            )),
            Event::Rule => result.push(block("divider", json!({"kind":"none"}), json!({}))),
            Event::Start(Tag::List(start)) => {
                let mut nesting = 1;
                let begin = position + 1;
                position += 1;
                while position < events.len() {
                    match events[position] {
                        Event::Start(Tag::List(_)) => nesting += 1,
                        Event::End(TagEnd::List(_)) => {
                            nesting -= 1;
                            if nesting == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    position += 1;
                }
                result.extend(list(
                    &events[begin..position],
                    &xmls,
                    start.is_some(),
                    depth,
                )?);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let begin = position + 1;
                position += 1;
                let mut nesting = 1;
                while position < events.len() {
                    match events[position] {
                        Event::Start(Tag::BlockQuote(_)) => nesting += 1,
                        Event::End(TagEnd::BlockQuote(_)) => {
                            nesting -= 1;
                            if nesting == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    position += 1;
                }
                result.push(block(
                    "quote",
                    container(inline::events(&events[begin..position], &xmls, depth)?),
                    json!({}),
                ));
            }
            Event::Start(Tag::Table(_)) => {
                position += 1;
                let mut rows: Vec<Vec<Value>> = vec![];
                let mut row = vec![];
                while position < events.len() && events[position] != Event::End(TagEnd::Table) {
                    match events[position] {
                        Event::Start(Tag::TableCell) => {
                            let begin = position + 1;
                            position += 1;
                            while position < events.len()
                                && events[position] != Event::End(TagEnd::TableCell)
                            {
                                position += 1;
                            }
                            row.push(container(inline::events(
                                &events[begin..position],
                                &xmls,
                                depth,
                            )?));
                        }
                        Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                            rows.push(std::mem::take(&mut row))
                        }
                        _ => {}
                    }
                    position += 1;
                }
                result.push(block(
                    "table",
                    json!({"kind":"table","rows":rows}),
                    json!({}),
                ));
            }
            Event::Text(token) => {
                if let Some(extension) = xmls.get(token.as_ref())
                    && !apply_extension(extension, &mut result, depth)?
                {
                    return Err("inline extension outside paragraph".into());
                }
            }
            _ => {}
        }
        position += 1;
    }
    Ok(result)
}

fn list(
    events: &[Event<'_>],
    xmls: &Extensions,
    ordered: bool,
    depth: usize,
) -> Result<Vec<NoteDocumentBlock>> {
    let mut result: Vec<NoteDocumentBlock> = vec![];
    let mut position = 0;
    while position < events.len() {
        if matches!(events[position], Event::Start(Tag::Item)) {
            let begin = position + 1;
            position += 1;
            let mut nesting = 1;
            while position < events.len() {
                match events[position] {
                    Event::Start(Tag::Item) => nesting += 1,
                    Event::End(TagEnd::Item) => {
                        nesting -= 1;
                        if nesting == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                position += 1;
            }
            let parts = &events[begin..position];
            let checked = parts.iter().find_map(|event| {
                if let Event::TaskListMarker(value) = event {
                    Some(*value)
                } else {
                    None
                }
            });
            let nested = parts
                .iter()
                .position(|event| matches!(event, Event::Start(Tag::List(_))));
            let mut item = block(
                if checked.is_some() {
                    "checkListItem"
                } else if ordered {
                    "numberedListItem"
                } else {
                    "bulletListItem"
                },
                container(inline::events(
                    &parts[..nested.unwrap_or(parts.len())],
                    xmls,
                    depth,
                )?),
                checked.map_or(json!({}), |value| json!({"checked":value})),
            );
            if let Some(begin) = nested {
                if depth >= 64 {
                    return Err("list depth limit".into());
                }
                let Event::Start(Tag::List(start)) = parts[begin] else {
                    unreachable!()
                };
                let mut end = begin + 1;
                let mut nesting = 1;
                while end < parts.len() {
                    match parts[end] {
                        Event::Start(Tag::List(_)) => nesting += 1,
                        Event::End(TagEnd::List(_)) => {
                            nesting -= 1;
                            if nesting == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    end += 1;
                }
                if end + 1 != parts.len() {
                    return Err("complex list item requires children XML".into());
                }
                item.children = list(&parts[begin + 1..end], xmls, start.is_some(), depth + 1)?;
            }
            result.push(item);
        }
        position += 1;
    }
    Ok(result)
}

fn apply_extension(
    extension: &Extension,
    result: &mut Vec<NoteDocumentBlock>,
    depth: usize,
) -> Result<bool> {
    match extension {
        Extension::Xml(element) => special::apply(element, result, depth),
        Extension::Math {
            source,
            display: true,
        } => {
            result.push(block(
                "formulaBlock",
                json!({"kind":"formula","source":source}),
                json!({}),
            ));
            Ok(true)
        }
        Extension::Math { .. } => Ok(false),
    }
}
