use super::*;
use crate::data_interface::plugin_resources::RowPolicy;

pub(super) fn provenance(
    tx: &Transaction<'_>,
    resources: &MetadataResources,
    item_id: &str,
    field_name: &str,
    provenance: &MetadataProvenanceV1,
) -> StepResult<()> {
    write(
        tx,
        resources,
        "provenance-upsert",
        vec![
            text(item_id),
            text(field_name),
            text(&provenance.source_kind),
            optional_text(provenance.source_reference.as_deref()),
            DataValue::Integer(i64::from(provenance.confirmed_by_user)),
        ],
    )?;
    Ok(())
}

pub(super) fn refresh(
    tx: &Transaction<'_>,
    resources: &MetadataResources,
    item_id: &str,
    policy: &MetadataDerivationPolicyV1,
) -> StepResult<()> {
    write(tx, resources, "search-delete", vec![text(item_id)])?;
    write(tx, resources, "search-insert", vec![text(item_id)])?;
    // Preserve the actual title/year read after FTS, including trigger changes.
    let rows = resources
        .read(
            tx,
            "derivation-title-year",
            vec![text(item_id)],
            RowPolicy::FirstRequired,
            &[ColumnCodec::String, ColumnCodec::OptionalI32],
        )
        .map_err(|error| ("derivation-title-year", MetadataProblem::Data(error)))?;
    let (title, year) = match rows.rows.as_slice() {
        [row] => match row.as_slice() {
            [
                DataValue::Text(title),
                year @ (DataValue::Null | DataValue::Integer(_)),
            ] => (title, year),
            _ => return Err(("derivation-title-year", MetadataProblem::Binding)),
        },
        _ => return Err(("derivation-title-year", MetadataProblem::Binding)),
    };
    let doi = ids(tx, resources, "duplicate-doi-ids", vec![text(item_id)])?;
    for candidate in doi {
        insert(
            tx,
            resources,
            item_id,
            &candidate,
            &policy.doi_reason,
            policy.doi_confidence,
        )?;
    }
    let normalized_title = match policy.title_rule {
        MetadataTitleRuleV1::UnicodeLowercase => title.to_lowercase(),
    };
    let candidates = ids(
        tx,
        resources,
        "duplicate-title-ids",
        vec![text(item_id), text(&normalized_title), year.clone()],
    )?;
    for candidate in candidates {
        insert(
            tx,
            resources,
            item_id,
            &candidate,
            &policy.title_reason,
            policy.title_confidence,
        )?;
    }
    Ok(())
}

fn ids(
    tx: &Transaction<'_>,
    resources: &MetadataResources,
    key: &'static str,
    arguments: Vec<DataValue>,
) -> StepResult<Vec<String>> {
    let rows = resources
        .read(tx, key, arguments, RowPolicy::All, &[ColumnCodec::String])
        .map_err(|error| (key, MetadataProblem::Data(error)))?;
    rows.rows
        .into_iter()
        .map(|row| match row.as_slice() {
            [DataValue::Text(id)] => Ok(id.clone()),
            _ => Err((key, MetadataProblem::Binding)),
        })
        .collect()
}

fn insert(
    tx: &Transaction<'_>,
    resources: &MetadataResources,
    item_id: &str,
    candidate: &str,
    reason: &str,
    confidence: f64,
) -> StepResult<()> {
    write(
        tx,
        resources,
        "duplicate-insert",
        vec![
            uuid(),
            text(item_id),
            text(candidate),
            text(reason),
            DataValue::Real(confidence),
        ],
    )?;
    Ok(())
}
