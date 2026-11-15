use super::*;

pub(super) fn markdown(source: &str, depth: usize) -> Result<Vec<Value>> {
    if depth > 64 {
        return Err("inline depth limit".into());
    }
    let (input, xmls) = preprocess(source)?;
    events(
        &Parser::new_ext(&input, options()).collect::<Vec<_>>(),
        &xmls,
        depth + 1,
    )
}

pub(super) fn events(events: &[Event<'_>], xmls: &Extensions, depth: usize) -> Result<Vec<Value>> {
    let mut result = vec![];
    let mut styles: Vec<&str> = vec![];
    let mut link: Option<(String, Vec<Value>)> = None;
    for event in events {
        let mut items = vec![];
        match event {
            Event::Start(Tag::Strong) => styles.push("bold"),
            Event::End(TagEnd::Strong) => {
                styles.pop();
            }
            Event::Start(Tag::Emphasis) => styles.push("italic"),
            Event::End(TagEnd::Emphasis) => {
                styles.pop();
            }
            Event::Start(Tag::Strikethrough) => styles.push("strike"),
            Event::End(TagEnd::Strikethrough) => {
                styles.pop();
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                if link.is_some() {
                    return Err("nested links".into());
                }
                link = Some((dest_url.to_string(), vec![]));
            }
            Event::End(TagEnd::Link) => {
                let (href, content) = link.take().ok_or("unmatched link")?;
                result.push(json!({"type":"link","href":href,"content":content}));
            }
            Event::Text(value) => {
                let mut remaining = value.as_ref();
                while !remaining.is_empty() {
                    let matched = xmls
                        .iter()
                        .filter_map(|(token, element)| {
                            remaining.find(token).map(|index| (index, token, element))
                        })
                        .min_by_key(|value| value.0);
                    if let Some((index, token, element)) = matched {
                        if index > 0 {
                            items.push(text(&remaining[..index], &styles));
                        }
                        let mut extension = match element {
                            Extension::Xml(element) => element_inline(element, depth + 1)?,
                            Extension::Math {
                                source,
                                display: false,
                            } => vec![json!({"type":"inlineFormula","source":source})],
                            Extension::Math { display: true, .. } => {
                                return Err("display formula must occupy its own paragraph".into());
                            }
                        };
                        for item in &mut extension {
                            if string(item, "type") == "text" {
                                let marks = item["styles"].as_array_mut().ok_or("invalid marks")?;
                                for style in &styles {
                                    if !marks.contains(&json!(style)) {
                                        marks.push(json!(style));
                                    }
                                }
                            }
                        }
                        items.extend(extension);
                        remaining = &remaining[index + token.len()..];
                    } else {
                        items.push(text(remaining, &styles));
                        break;
                    }
                }
            }
            Event::Code(value) => {
                let mut marks = styles.clone();
                marks.push("code");
                items.push(text(value, &marks));
            }
            Event::InlineMath(value) => {
                items.push(json!({"type":"inlineFormula","source":value.as_ref()}))
            }
            Event::SoftBreak | Event::HardBreak => items.push(text("\n", &styles)),
            Event::Start(Tag::Image { .. }) => {
                return Err("use image XML for note image blocks".into());
            }
            Event::Html(_) | Event::InlineHtml(_) => return Err("unparsed XML".into()),
            _ => {}
        }
        if let Some((_, content)) = &mut link {
            if items.iter().any(|item| string(item, "type") != "text") {
                return Err("links may contain only styled text".into());
            }
            content.extend(items);
        } else {
            result.extend(items);
        }
    }
    if link.is_some() {
        return Err("unclosed link".into());
    }
    Ok(super::super::merge_text_items(result))
}

fn element_inline(element: &Element, depth: usize) -> Result<Vec<Value>> {
    if depth > 64 {
        return Err("inline depth limit".into());
    }
    match element.name.as_str() {
        "reference" => {
            if !element.attr("display").is_empty() {
                return Err("block reference inside paragraph".into());
            }
            let kind = element.attr("kind");
            let id = super::super::reference_id(element.attr("kind"), element.attr("target"))?;
            let content = markdown(&xml::inner(element), depth + 1)?;
            if content.iter().any(|item| string(item, "type") != "text") {
                return Err("reference labels contain only styled text".into());
            }
            let label = content
                .iter()
                .map(|item| string(item, "text"))
                .collect::<String>();
            Ok(vec![
                json!({"type":"mention","targetKind":kind,"targetId":id,"labelSnapshot":label,"content":content}),
            ])
        }
        "u" | "span" | "strong" | "em" | "s" => {
            let mut content = markdown(&xml::inner(element), depth + 1)?;
            for item in &mut content {
                let targets = if string(item, "type") == "link" || string(item, "type") == "mention"
                {
                    item["content"]
                        .as_array_mut()
                        .ok_or("missing styled label")?
                } else {
                    if string(item, "type") != "text" {
                        return Err("style tags may wrap only text or labels".into());
                    }
                    apply_style(item, element)?;
                    continue;
                };
                for item in targets {
                    apply_style(item, element)?;
                }
            }
            Ok(content)
        }
        _ => Err(format!("{} is a block-only XML tag", element.name)),
    }
}
fn apply_style(item: &mut Value, element: &Element) -> Result<()> {
    if string(item, "type") != "text" {
        return Err("invalid styled text".into());
    }
    if let Some(style) = match element.name.as_str() {
        "u" => Some("underline"),
        "strong" => Some("bold"),
        "em" => Some("italic"),
        "s" => Some("strike"),
        _ => None,
    } {
        let styles = item["styles"].as_array_mut().ok_or("missing styles")?;
        if !styles.contains(&json!(style)) {
            styles.push(json!(style));
        }
    }
    for (attribute, key) in [("color", "textColor"), ("background", "backgroundColor")] {
        if !element.attr(attribute).is_empty() {
            item[key] = json!(element.attr(attribute));
        }
    }
    Ok(())
}
