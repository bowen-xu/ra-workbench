//! Protect authoring extensions before CommonMark consumes backslash escapes.
use super::*;
use std::{collections::BTreeSet, ops::Range};

pub(super) enum Extension {
    Xml(Element),
    Math { source: String, display: bool },
}
pub(super) type Extensions = BTreeMap<String, Extension>;

pub(super) fn preprocess(source: &str) -> Result<(String, Extensions)> {
    let mut protected: Vec<Range<usize>> = vec![];
    let mut text_ranges = vec![];
    let mut xml_starts = BTreeSet::new();
    let mut code_start = None;
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_start.take() {
                    protected.push(start..range.end);
                }
            }
            Event::Code(_) | Event::InlineMath(_) | Event::DisplayMath(_) => protected.push(range),
            Event::Text(_) => text_ranges.push(range),
            Event::Html(_) | Event::InlineHtml(_) => {
                xml_starts.insert(range.start);
            }
            _ => {}
        }
    }
    protected.sort_by_key(|range| range.start);
    let prefix = format!("RAEXT{}X", uuid::Uuid::new_v4().simple());
    let mut extensions = Extensions::new();
    let mut output = String::new();
    let mut cursor = 0;
    let mut copied = 0;
    let mut protected_index = 0;
    let bytes = source.as_bytes();
    let closes = closing_positions(bytes);
    while cursor < bytes.len() {
        while protected_index < protected.len() && protected[protected_index].end <= cursor {
            protected_index += 1;
        }
        if let Some(range) = protected.get(protected_index)
            && range.contains(&cursor)
        {
            cursor = range.end;
            continue;
        }
        let found = if xml_starts.contains(&cursor) {
            let (element, length) = xml::fragment(&source[cursor..])?;
            Some((Extension::Xml(element), cursor + length))
        } else if bytes[cursor] == b'\\' && cursor + 1 < bytes.len() {
            let display = match bytes[cursor + 1] {
                b'(' => Some(false),
                b'[' => Some(true),
                _ => None,
            };
            // Only text can open math. Destinations/attributes never become
            // formulas; styled link labels still pass through XML processing.
            let in_text = text_ranges
                .get(text_ranges.partition_point(|range| range.end <= cursor + 1))
                .is_some_and(|range| range.contains(&(cursor + 1)));
            if let Some(display) = display.filter(|_| in_text) {
                closes[usize::from(display)]
                    .get(closes[usize::from(display)].partition_point(|end| *end < cursor + 2))
                    .copied()
                    .map(|end| {
                        (
                            Extension::Math {
                                source: formula_source(&source[cursor + 2..end], display)
                                    .to_owned(),
                                display,
                            },
                            end + 2,
                        )
                    })
            } else {
                // An escaped backslash cannot open a formula.
                cursor += 2;
                continue;
            }
        } else {
            None
        };
        if let Some((extension, end)) = found {
            let token = format!("{prefix}{}TOKEN", extensions.len());
            output.push_str(&source[copied..cursor]);
            output.push_str(&token);
            extensions.insert(token, extension);
            cursor = end;
            copied = end;
        } else {
            cursor += 1;
        }
    }
    output.push_str(&source[copied..]);
    Ok((output, extensions))
}

// Index closers once so unmatched openers cannot repeatedly scan the note.
fn closing_positions(bytes: &[u8]) -> [Vec<usize>; 2] {
    let mut positions = [vec![], vec![]];
    let mut cursor = 0;
    while cursor + 1 < bytes.len() {
        if bytes[cursor] == b'\\' {
            match bytes[cursor + 1] {
                b')' => positions[0].push(cursor),
                b']' => positions[1].push(cursor),
                _ => {}
            }
            cursor += 2;
        } else {
            cursor += 1;
        }
    }
    positions
}

fn formula_source(source: &str, display: bool) -> &str {
    if !display {
        return source;
    }
    let source = source
        .strip_prefix("\r\n")
        .or_else(|| source.strip_prefix('\n'))
        .unwrap_or(source);
    source
        .strip_suffix("\r\n")
        .or_else(|| source.strip_suffix('\n'))
        .unwrap_or(source)
}
