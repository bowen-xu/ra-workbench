use super::Result;
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(super) struct Element {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub content: Vec<Part>,
}
#[derive(Clone, Debug)]
pub(super) enum Part {
    Text(String),
    Element(Element),
}

impl Element {
    pub fn attr(&self, key: &str) -> &str {
        self.attrs.get(key).map_or("", String::as_str)
    }
    pub fn text(&self) -> Result<String> {
        self.content
            .iter()
            .map(|part| match part {
                Part::Text(text) => Ok(text.clone()),
                Part::Element(_) => Err("nested XML is not allowed here".into()),
            })
            .collect()
    }
}

fn start(event: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<Element> {
    let name = std::str::from_utf8(event.name().as_ref())
        .map_err(|_| "invalid XML name")?
        .to_owned();
    let definition = &*super::protocol::DEFINITION;
    let allowed = &definition
        .tags
        .get(&name)
        .ok_or_else(|| format!("unknown XML tag: {name}"))?
        .attributes;
    let mut attrs = BTreeMap::new();
    for attribute in event.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        let key = std::str::from_utf8(attribute.key.as_ref())
            .map_err(|_| "invalid attribute")?
            .to_owned();
        if !allowed.contains(&key) {
            return Err(format!("unknown {name} attribute: {key}"));
        }
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|error| error.to_string())?
            .into_owned();
        if attrs.insert(key, value).is_some() {
            return Err("duplicate XML attribute".into());
        }
    }
    for key in ["color", "background"] {
        if let Some(value) = attrs.get(key)
            && !definition.colors.contains(value)
        {
            return Err("unsupported color".into());
        }
    }
    if attrs
        .get("align")
        .is_some_and(|value| !definition.alignments.contains(value))
    {
        return Err("unsupported alignment".into());
    }
    Ok(Element {
        name,
        attrs,
        content: vec![],
    })
}

pub(super) fn fragment(input: &str) -> Result<(Element, usize)> {
    let mut reader = Reader::from_str(input);
    let mut stack: Vec<Element> = vec![];
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(event) => {
                stack.push(start(&event, &reader)?);
                if stack.len() > 64 {
                    return Err("XML depth limit".into());
                }
            }
            Event::Empty(event) => {
                let element = start(&event, &reader)?;
                if let Some(parent) = stack.last_mut() {
                    parent.content.push(Part::Element(element));
                } else {
                    return Ok((element, reader.buffer_position() as usize));
                }
            }
            Event::End(_) => {
                let element = stack.pop().ok_or("unmatched XML close tag")?;
                if let Some(parent) = stack.last_mut() {
                    parent.content.push(Part::Element(element));
                } else {
                    return Ok((element, reader.buffer_position() as usize));
                }
            }
            Event::Text(event) => {
                stack
                    .last_mut()
                    .ok_or("text outside XML")?
                    .content
                    .push(Part::Text(
                        event
                            .decode()
                            .map_err(|error| error.to_string())?
                            .into_owned(),
                    ));
            }
            Event::CData(event) => {
                stack
                    .last_mut()
                    .ok_or("CDATA outside XML")?
                    .content
                    .push(Part::Text(
                        event
                            .decode()
                            .map_err(|error| error.to_string())?
                            .into_owned(),
                    ));
            }
            Event::GeneralRef(event) => {
                let value = format!("&{};", event.decode().map_err(|error| error.to_string())?);
                stack
                    .last_mut()
                    .ok_or("entity outside XML")?
                    .content
                    .push(Part::Text(
                        quick_xml::escape::unescape(&value)
                            .map_err(|error| error.to_string())?
                            .into_owned(),
                    ));
            }
            _ => {
                return Err(
                    "XML comments, declarations, DTD and incomplete fragments are not allowed"
                        .into(),
                );
            }
        }
    }
}

pub(super) fn inner(element: &Element) -> String {
    element
        .content
        .iter()
        .map(|part| match part {
            Part::Text(text) => text.clone(),
            Part::Element(child) => {
                let attrs = child
                    .attrs
                    .iter()
                    .map(|(name, value)| format!(" {name}=\"{}\"", super::render::escape(value)))
                    .collect::<String>();
                format!(
                    "<{}{attrs}>{}</{}>",
                    child.name,
                    super::render::escape(&inner(child)),
                    child.name
                )
            }
        })
        .collect()
}
