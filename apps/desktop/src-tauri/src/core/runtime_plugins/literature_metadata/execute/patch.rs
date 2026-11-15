use super::*;

pub(super) fn apply(
    tx: &Transaction<'_>,
    resources: &MetadataResources,
    id: &str,
    prepared: &DeferredMetadataPatchV1,
) -> StepResult<()> {
    let (patch, doi) = match prepared {
        DeferredMetadataPatchV1::Rejected { error } => {
            return Err(("patch", MetadataProblem::Domain(error.clone())));
        }
        DeferredMetadataPatchV1::Ready { patch, doi } => (patch, doi),
    };
    use LiteratureMetadataPatch::*;
    let scalar = match patch {
        ItemType(value) => Some(("patch-item-type", text(value))),
        Title(value) => Some(("patch-title", text(value))),
        AbstractText(value) => Some(("patch-abstract-text", text(value))),
        IssuedYear(value) => Some(("patch-issued-year", year(*value))),
        ContainerTitle(value) => Some(("patch-container-title", text(value))),
        ConferenceName(value) => Some(("patch-conference-name", text(value))),
        ProceedingsTitle(value) => Some(("patch-proceedings-title", text(value))),
        Place(value) => Some(("patch-place", text(value))),
        Volume(value) => Some(("patch-volume", text(value))),
        Issue(value) => Some(("patch-issue", text(value))),
        Pages(value) => Some(("patch-pages", text(value))),
        Url(value) => Some(("patch-url", text(value))),
        Language(value) => Some(("patch-language", text(value))),
        BibtexKey(value) => Some(("patch-bibtex-key", text(value))),
        Doi(_) | Creators(_) | Tags(_) => None,
    };
    if let Some((key, value)) = scalar {
        write(tx, resources, key, vec![text(id), value])?;
        return Ok(());
    }
    match patch {
        Creators(creators) => {
            write(tx, resources, "creators-delete", vec![text(id)])?;
            for (position, creator) in creators.iter().enumerate() {
                let creator_id = uuid();
                write(
                    tx,
                    resources,
                    "creator-insert",
                    vec![
                        creator_id.clone(),
                        text(&creator.family_name),
                        text(&creator.given_name),
                        text(&creator.literal_name),
                    ],
                )?;
                // As in the existing save adapter, no process can allocate i64::MAX creators.
                let position = i64::try_from(position)
                    .map_err(|_| ("item-creator-insert", MetadataProblem::Binding))?;
                write(
                    tx,
                    resources,
                    "item-creator-insert",
                    vec![
                        text(id),
                        creator_id,
                        text(&creator.role),
                        DataValue::Integer(position),
                    ],
                )?;
            }
        }
        Doi(_) => {
            write(tx, resources, "doi-delete", vec![text(id)])?;
            if let Some(doi) = doi {
                write(
                    tx,
                    resources,
                    "doi-insert",
                    vec![uuid(), text(id), text(&doi.normalized), text(&doi.display)],
                )?;
            }
        }
        Tags(tags) => {
            write(tx, resources, "tags-delete", vec![text(id)])?;
            for tag in tags {
                write(tx, resources, "tag-insert", vec![uuid(), text(tag)])?;
                write(tx, resources, "item-tag-insert", vec![text(id), text(tag)])?;
            }
        }
        _ => unreachable!("scalar patches returned above"),
    }
    Ok(())
}
