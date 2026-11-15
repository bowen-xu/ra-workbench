use super::{Result, parse, render_block};
use crate::notes::{NoteDocument, NoteDocumentBlock};

pub(crate) fn candidate(before: &NoteDocument, source: &str) -> Result<(String, NoteDocument)> {
    let (title, mut after) = parse(&before.page_id, source)?;
    reconcile(&before.blocks, &mut after.blocks)?;
    super::validate_candidate(&after, before)?;
    Ok((title, after))
}

pub(super) fn reconcile(
    before: &[NoteDocumentBlock],
    after: &mut [NoteDocumentBlock],
) -> Result<()> {
    let original = before
        .iter()
        .map(render_block)
        .collect::<Result<Vec<_>>>()?;
    let rendered = after.iter().map(render_block).collect::<Result<Vec<_>>>()?;
    let single_position_edit = original.len() == rendered.len()
        && original
            .iter()
            .zip(&rendered)
            .filter(|(before, after)| before != after)
            .count()
            == 1;
    if original != rendered
        && !single_position_edit
        && original
            .iter()
            .any(|value| original.iter().filter(|other| *other == value).count() > 1)
    {
        return Err(
            "repeated blocks in the affected range have ambiguous identity; narrow the patch"
                .into(),
        );
    }
    let mut used = std::collections::BTreeSet::new();
    let mut mapped = vec![None; after.len()];
    // Equal positional blocks are untouched, including repeated empty paragraphs.
    for (index, value) in rendered.iter().enumerate() {
        if original.get(index) == Some(value) {
            mapped[index] = Some(index);
            used.insert(index);
        }
    }
    for (index, value) in rendered.iter().enumerate() {
        if mapped[index].is_some() {
            continue;
        }
        let matches = original
            .iter()
            .enumerate()
            .filter(|(position, text)| !used.contains(position) && *text == value)
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err("block identity is ambiguous; use a smaller patch".into());
        }
        if let Some(position) = matches.first() {
            mapped[index] = Some(*position);
            used.insert(*position);
        }
    }
    let remaining = before
        .iter()
        .enumerate()
        .filter(|(position, _)| !used.contains(position))
        .map(|(position, _)| position)
        .collect::<Vec<_>>();
    let mut remaining = remaining.into_iter();
    for (index, block) in after.iter_mut().enumerate() {
        let original_index = mapped[index].or_else(|| remaining.next());
        if let Some(position) = original_index {
            let prior = &before[position];
            if original[position] == rendered[index] {
                *block = prior.clone();
                continue;
            }
            if block.block_type == "unsupported" {
                return Err("opaque blocks must be preserved unchanged".into());
            }
            if prior.block_type == "unsupported"
                || !crate::notes::document_validation::known_block(&prior.block_type)
            {
                continue;
            }
            block.id = prior.id.clone();
            let mut props = prior.props.clone();
            for key in ["textAlignment", "textColor", "backgroundColor"] {
                props
                    .as_object_mut()
                    .ok_or("invalid prior props")?
                    .remove(key);
            }
            // Toggle mode is authorable on headings. Do not treat the same
            // name on an unrelated future block as a known presentation prop.
            if prior.block_type == "heading" || block.block_type == "heading" {
                props
                    .as_object_mut()
                    .ok_or("invalid prior props")?
                    .remove("isToggleable");
            }
            if block.block_type != prior.block_type {
                for key in ["level", "checked", "language"] {
                    props
                        .as_object_mut()
                        .ok_or("invalid prior props")?
                        .remove(key);
                }
            }
            for (key, value) in block.props.as_object().ok_or("invalid block props")? {
                props[key] = value.clone();
            }
            block.props = props;
            reconcile(&prior.children, &mut block.children)?;
            if block.block_type == prior.block_type {
                if prior.block_type == "codeBlock"
                    && prior.content["items"].as_array().is_some_and(|items| {
                        items.iter().any(|item| {
                            item["styles"]
                                .as_array()
                                .is_none_or(|styles| !styles.is_empty())
                                || item.get("textColor").is_some()
                                || item.get("backgroundColor").is_some()
                        })
                    })
                {
                    return Err(
                        "styled code cannot be edited losslessly through a code fence".into(),
                    );
                }
                super::inline_mapping::content(&prior.content, &mut block.content)?;
            }
        }
    }
    Ok(())
}
