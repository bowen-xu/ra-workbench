//! Execute the original sequence; all values and domain choices come from the plan.
use super::SaveResources;
use crate::data_interface::plugin_resources::PluginDataError;
use research_workbench_native_sdk::{catalog_save::CatalogSavePlan, data::DataValue};
use rusqlite::Transaction;

type Result<T> = std::result::Result<T, Box<PluginDataError>>;

fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}

fn year(value: Option<i32>) -> DataValue {
    value.map_or(DataValue::Null, |value| {
        DataValue::Integer(i64::from(value))
    })
}

fn uuid() -> DataValue {
    DataValue::Text(uuid::Uuid::new_v4().to_string())
}

pub(crate) fn save(
    tx: &Transaction<'_>,
    resources: &SaveResources,
    plan: &CatalogSavePlan,
    item_id: &str,
) -> Result<()> {
    let item = &plan.item;
    resources.item_upsert.write(
        tx,
        vec![
            text(item_id),
            text(&item.item_type),
            text(&item.title),
            text(&item.abstract_text),
            year(item.issued_year),
            text(&item.container_title),
            text(&item.conference_name),
            text(&item.proceedings_title),
            text(&item.place),
            text(&item.volume),
            text(&item.issue),
            text(&item.pages),
            text(&item.url),
            text(&item.language),
            text(&item.bibtex_key),
            item.raw_bibtex.as_deref().map_or(DataValue::Null, text),
        ],
    )?;
    resources.creators_delete.write(tx, vec![text(item_id)])?;
    for (position, creator) in plan.creators.iter().enumerate() {
        let creator_id = uuid();
        resources.creator_insert.write(
            tx,
            vec![
                creator_id.clone(),
                text(&creator.family_name),
                text(&creator.given_name),
                text(&creator.literal_name),
            ],
        )?;
        // A Vec of creators cannot reach i64::MAX elements in this process.
        let position = i64::try_from(position).map_err(|_| {
            resources
                .item_creator_insert
                .parameter_failure("catalog_creator_position_range")
        })?;
        resources.item_creator_insert.write(
            tx,
            vec![
                text(item_id),
                creator_id,
                text(&creator.role),
                DataValue::Integer(position),
            ],
        )?;
    }
    resources.doi_delete.write(tx, vec![text(item_id)])?;
    if let Some(doi) = &plan.doi {
        resources.doi_insert.write(
            tx,
            vec![
                uuid(),
                text(item_id),
                text(&doi.normalized),
                text(&doi.display),
            ],
        )?;
    }
    resources.tags_delete.write(tx, vec![text(item_id)])?;
    for tag in &plan.tags {
        resources.tag_insert.write(tx, vec![uuid(), text(tag)])?;
        resources
            .item_tag_insert
            .write(tx, vec![text(item_id), text(tag)])?;
    }
    for field in &plan.provenance.fields {
        resources.provenance_upsert.write(
            tx,
            vec![
                text(item_id),
                text(field),
                text(&plan.provenance.source_kind),
                DataValue::Integer(i64::from(plan.provenance.confirmed_by_user)),
            ],
        )?;
    }
    refresh_search(tx, resources, item_id)?;
    duplicates(tx, resources, plan, item_id)
}

fn duplicates(
    tx: &Transaction<'_>,
    resources: &SaveResources,
    plan: &CatalogSavePlan,
    item_id: &str,
) -> Result<()> {
    // Each complete SELECT precedes its INSERT loop, including original repeats.
    let doi_candidates = resources.duplicate_doi_ids.ids(tx, vec![text(item_id)])?;
    for candidate in doi_candidates {
        resources.duplicate_insert.write(
            tx,
            vec![
                uuid(),
                text(item_id),
                text(&candidate),
                text(&plan.duplicates.doi_reason),
                DataValue::Real(plan.duplicates.doi_confidence),
            ],
        )?;
    }
    let title_candidates = resources.duplicate_title_ids.ids(
        tx,
        vec![
            text(item_id),
            text(&plan.duplicates.normalized_title),
            year(plan.item.issued_year),
        ],
    )?;
    for candidate in title_candidates {
        resources.duplicate_insert.write(
            tx,
            vec![
                uuid(),
                text(item_id),
                text(&candidate),
                text(&plan.duplicates.title_reason),
                DataValue::Real(plan.duplicates.title_confidence),
            ],
        )?;
    }
    Ok(())
}

/// The original FTS refresh also follows browser-specific identifiers in the same transaction.
pub(crate) fn refresh_search(
    tx: &Transaction<'_>,
    resources: &SaveResources,
    item_id: &str,
) -> Result<()> {
    resources.search_delete.write(tx, vec![text(item_id)])?;
    resources.search_insert.write(tx, vec![text(item_id)])?;
    Ok(())
}
