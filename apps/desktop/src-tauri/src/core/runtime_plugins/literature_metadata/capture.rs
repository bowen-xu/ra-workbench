//! Owned observations of the original candidate reads. No guard crosses the host wait.
use super::{
    MetadataFailure, MetadataPhaseOutcome as Outcome, MetadataProblem as Problem, MetadataResources,
};
use crate::{
    data_interface::plugin_resources::{ColumnCodec, PluginDataError, RowPolicy},
    runtime_plugins::literature_catalog::data::{CatalogResources, capture_items},
};
use research_workbench_native_sdk::{
    catalog::{CatalogItem, CatalogItemRows, CatalogProjectionInput, CatalogRequest},
    catalog_save::CatalogDraft,
    data::{DataRows, DataValue},
    metadata::*,
};
use rusqlite::Connection;

mod inputs;

type Result<T> = std::result::Result<T, MetadataFailure>;

#[derive(Clone)]
pub(crate) struct MetadataCapture {
    request: MetadataRequestV1,
    header_rows: Option<DataRows>,
    candidate: Option<CatalogDraft>,
    header: Option<MetadataCandidateHeaderV1>,
    current_rows: Option<Vec<CatalogItemRows>>,
}

impl MetadataCapture {
    pub(crate) fn projection(&self) -> Option<CatalogProjectionInput> {
        Some(CatalogProjectionInput {
            request: CatalogRequest::Get {
                id: self.header.as_ref()?.item_id.clone()?,
            },
            items: self.current_rows.clone()?,
        })
    }

    pub(crate) fn input(&self, current: Option<CatalogItem>) -> Result<MetadataInputV1> {
        inputs::prepare(self, current)
    }
}

pub(crate) fn capture(
    db: &Connection,
    resources: &MetadataResources,
    catalog: &CatalogResources,
    request: MetadataRequestV1,
) -> Result<MetadataCapture> {
    let mut observed = MetadataCapture {
        request,
        header_rows: None,
        candidate: None,
        header: None,
        current_rows: None,
    };
    match &observed.request {
        MetadataRequestV1::FullApply { candidate_id, .. }
        | MetadataRequestV1::FieldsApply { candidate_id, .. } => {
            let rows = read_header(db, resources, candidate_id)?;
            let row = rows.rows.first().ok_or_else(missing)?;
            let header = MetadataCandidateHeaderV1 {
                item_id: optional_text(&row[1]),
                source_kind: text(&row[2]),
                source_reference: optional_text(&row[3]),
                import_result_id: optional_text(&row[4]),
            };
            // Preserve the original required link check before the JSON codec.
            if matches!(observed.request, MetadataRequestV1::FieldsApply { .. })
                && header.item_id.is_none()
            {
                return Err(domain(MetadataRejectionV1::InvalidFields {
                    reason: "候选未关联已有文献".into(),
                }));
            }
            let candidate = serde_json::from_str(&text(&row[0])).map_err(|cause| {
                let category = format!("{:?}", cause.classify());
                MetadataFailure::new(
                    "candidate-codec",
                    Problem::Connector(format!(
                        "metadata_candidate_json_{category}: line {}, column {}",
                        cause.line(),
                        cause.column()
                    )),
                    Outcome::NotStarted,
                )
            })?;
            if let MetadataRequestV1::FieldsApply { .. } = observed.request {
                observed.current_rows = Some(
                    capture_items(db, catalog, &[header.item_id.clone().ok_or_else(binding)?])
                        .map_err(data)?,
                );
            }
            observed.header_rows = Some(rows);
            observed.header = Some(header);
            observed.candidate = Some(candidate);
        }
        MetadataRequestV1::Reject { candidate_id } => {
            observed.header_rows = Some(read_reject(db, resources, candidate_id)?);
        }
        MetadataRequestV1::Update { .. } | MetadataRequestV1::Confirm { .. } => {}
    }
    Ok(observed)
}

/// Compare complete typed rows, including absent rows versus nullable values.
/// No changed JSON is interpreted, and no writes occur before this check.
pub(crate) fn recheck(
    db: &Connection,
    resources: &MetadataResources,
    catalog: &CatalogResources,
    observed: &MetadataCapture,
) -> Result<()> {
    let current = match &observed.request {
        MetadataRequestV1::FullApply { candidate_id, .. }
        | MetadataRequestV1::FieldsApply { candidate_id, .. } => {
            Some(read_header(db, resources, candidate_id)?)
        }
        MetadataRequestV1::Reject { candidate_id } => {
            Some(read_reject(db, resources, candidate_id)?)
        }
        _ => None,
    };
    if current != observed.header_rows {
        let resource = match observed.request {
            MetadataRequestV1::Reject { .. } => "literature.metadata.reject-import-result",
            _ => "literature.metadata.candidate-header",
        };
        return Err(changed(resources, resource));
    }
    if let Some(previous) = &observed.current_rows {
        let id = observed
            .header
            .as_ref()
            .and_then(|header| header.item_id.as_ref())
            .ok_or_else(binding)?;
        let current = capture_items(db, catalog, std::slice::from_ref(id)).map_err(data)?;
        if current != *previous {
            return Err(changed(resources, catalog_difference(previous, &current)));
        }
    }
    Ok(())
}

fn read_header(db: &Connection, resources: &MetadataResources, id: &str) -> Result<DataRows> {
    resources
        .read(
            db,
            "candidate-header",
            vec![DataValue::Text(id.into())],
            RowPolicy::FirstOptional,
            &[
                ColumnCodec::String,
                ColumnCodec::OptionalString,
                ColumnCodec::String,
                ColumnCodec::OptionalString,
                ColumnCodec::OptionalString,
            ],
        )
        .map_err(data)
}

fn read_reject(db: &Connection, resources: &MetadataResources, id: &str) -> Result<DataRows> {
    resources
        .read(
            db,
            "reject-import-result",
            vec![DataValue::Text(id.into())],
            RowPolicy::FirstOptional,
            &[ColumnCodec::OptionalString],
        )
        .map_err(data)
}

fn text(value: &DataValue) -> String {
    match value {
        DataValue::Text(value) => value.clone(),
        _ => unreachable!("fixed String codec validates every row before returning it"),
    }
}

fn optional_text(value: &DataValue) -> Option<String> {
    match value {
        DataValue::Null => None,
        value => Some(text(value)),
    }
}

fn domain(error: MetadataRejectionV1) -> MetadataFailure {
    MetadataFailure::new("capture", Problem::Domain(error), Outcome::NotStarted)
}

fn missing() -> MetadataFailure {
    domain(MetadataRejectionV1::NotFound {})
}

fn binding() -> MetadataFailure {
    MetadataFailure::new("capture", Problem::Binding, Outcome::NotStarted)
}

fn data(error: Box<PluginDataError>) -> MetadataFailure {
    MetadataFailure::new("capture", Problem::Data(error), Outcome::NotStarted)
}

fn catalog_difference(previous: &[CatalogItemRows], current: &[CatalogItemRows]) -> &'static str {
    if let (Some(previous), Some(current)) = (previous.first(), current.first())
        && previous.main == current.main
        && let (Some(previous), Some(current)) = (&previous.related, &current.related)
    {
        for (changed, resource) in [
            (
                previous.creators != current.creators,
                "literature.catalog.item-creators",
            ),
            (
                previous.tags != current.tags,
                "literature.catalog.item-tags",
            ),
            (
                previous.attachments != current.attachments,
                "literature.catalog.item-attachments",
            ),
            (
                previous.project_ids != current.project_ids,
                "literature.catalog.item-project-ids",
            ),
            (
                previous.metadata_sources != current.metadata_sources,
                "literature.catalog.item-metadata-sources",
            ),
        ] {
            if changed {
                return resource;
            }
        }
    }
    "literature.catalog.item-main"
}

fn changed(resources: &MetadataResources, resource: &'static str) -> MetadataFailure {
    MetadataFailure::new(
        "recheck",
        Problem::Data(Box::new(PluginDataError {
            code: "plugin_data_snapshot_changed",
            message: "元数据审阅所依据的候选或文献已变化；本次未写入。",
            package_key: resources.package_key().into(),
            resource_id: resource.into(),
            resource_version: 1,
            phase: "recheck",
            safe_cause: "metadata_observations_changed".into(),
            sqlite_code: None,
            sqlite_extended_code: None,
            original_sql_failure: None,
        })),
        Outcome::NotStarted,
    )
}
