//! Application patch: retain every source fragment when paragraphs are merged.
use crate::layout::Region;
use docling_core::{tree::TreeProv, unescape_text, Node};

pub(crate) fn fragment(region: &Region, start: usize, end: usize) -> TreeProv {
    TreeProv {
        page_no: 0, // The collector stamps the real selected page number.
        bbox: [region.l, region.t, region.r, region.b].map(f64::from),
        bottom_left: false,
        charspan: [start, end],
    }
}

pub(crate) fn stamp(node: &mut Node, page_no: usize) {
    match node {
        Node::TextProvenance { fragments, inner } => {
            for fragment in fragments {
                if fragment.page_no == 0 {
                    fragment.page_no = page_no;
                }
            }
            stamp(inner, page_no);
        }
        Node::Located { inner, .. } | Node::Furniture { inner, .. } => stamp(inner, page_no),
        Node::Group { children, .. } | Node::PictureChildren(children) => {
            for child in children {
                stamp(child, page_no);
            }
        }
        _ => {}
    }
}

pub(crate) fn rotate(fragments: &mut [TreeProv], rotation: u16, width: f64, height: f64) {
    for fragment in fragments {
        let [l, t, r, b] = fragment.bbox;
        fragment.bbox = match rotation {
            90 => [height - b, l, height - t, r],
            180 => [width - r, height - b, width - l, height - t],
            270 => [t, width - r, b, width - l],
            _ => [l, t, r, b],
        };
    }
}

pub(crate) fn paragraph(node: &Node) -> Option<&str> {
    match node {
        Node::Paragraph { text } => Some(text),
        Node::Located { inner, .. } | Node::TextProvenance { inner, .. } => paragraph(inner),
        _ => None,
    }
}

fn fragments(node: &Node) -> Vec<TreeProv> {
    match node {
        Node::TextProvenance { fragments, .. } => fragments.clone(),
        Node::Located { inner, .. } => fragments(inner),
        _ => Vec::new(),
    }
}

fn replace_text(node: &Node, text: String) -> Node {
    match node {
        Node::Located { location, inner } => Node::Located {
            location: *location,
            inner: Box::new(replace_text(inner, text)),
        },
        Node::TextProvenance { inner, .. } => replace_text(inner, text),
        _ => Node::Paragraph { text },
    }
}

pub(crate) fn merge(head: &Node, tail: &Node, text: String, hyphenated: bool) -> Node {
    let head_len = unescape_text(paragraph(head).unwrap_or_default().trim_end())
        .chars()
        .count()
        .saturating_sub(usize::from(hyphenated));
    let tail_text = unescape_text(paragraph(tail).unwrap_or_default());
    let trim = tail_text.chars().count() - tail_text.trim_start().chars().count();
    let offset = head_len + usize::from(!hyphenated);
    let mut provenance = fragments(head);
    for fragment in &mut provenance {
        fragment.charspan = fragment.charspan.map(|p| p.min(head_len));
    }
    for mut fragment in fragments(tail) {
        fragment.charspan = fragment.charspan.map(|p| offset + p.saturating_sub(trim));
        provenance.push(fragment);
    }
    let inner = replace_text(head, text);
    if provenance.is_empty() {
        inner
    } else {
        Node::TextProvenance {
            fragments: provenance,
            inner: Box::new(inner),
        }
    }
}
