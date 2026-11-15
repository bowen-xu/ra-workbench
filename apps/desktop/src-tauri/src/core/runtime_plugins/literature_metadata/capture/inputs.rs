use super::*;
use research_workbench_native_sdk::catalog_save::PreparedDoi;

pub(super) fn prepare(
    captured: &MetadataCapture,
    current: Option<CatalogItem>,
) -> Result<MetadataInputV1> {
    Ok(match &captured.request {
        MetadataRequestV1::Update { item_id, patch } => MetadataInputV1::Update {
            input: Box::new(MetadataUpdateInputV1 {
                item_id: item_id.clone(),
                patch: patch.clone(),
                doi: match patch {
                    LiteratureMetadataPatch::Doi(value) => Some(doi(value)),
                    _ => None,
                },
            }),
        },
        MetadataRequestV1::Confirm {
            item_id,
            field_names,
        } => MetadataInputV1::Confirm {
            input: Box::new(MetadataConfirmInputV1 {
                item_id: item_id.clone(),
                field_names: field_names.clone(),
            }),
        },
        MetadataRequestV1::FullApply {
            candidate_id,
            draft,
        } => {
            let candidate = captured.candidate.clone().ok_or_else(binding)?;
            MetadataInputV1::FullApply {
                input: Box::new(MetadataFullApplyInputV1 {
                    candidate_id: candidate_id.clone(),
                    header: captured.header.clone().ok_or_else(binding)?,
                    candidate_normalized_doi: crate::doi::normalized_doi(&candidate.doi),
                    candidate,
                    draft: (**draft).clone(),
                    doi: doi(&draft.doi),
                }),
            }
        }
        MetadataRequestV1::FieldsApply {
            candidate_id,
            decisions,
            candidate_trust,
        } => {
            let candidate = captured.candidate.clone().ok_or_else(binding)?;
            let current = current.ok_or_else(missing)?;
            MetadataInputV1::FieldsApply {
                input: Box::new(MetadataFieldsApplyInputV1 {
                    candidate_id: candidate_id.clone(),
                    header: captured.header.clone().ok_or_else(binding)?,
                    candidate_doi: doi(&candidate.doi),
                    candidate,
                    current_normalized_doi: crate::doi::normalized_doi(&current.doi),
                    current,
                    decisions: decisions.clone(),
                    candidate_trust: *candidate_trust,
                }),
            }
        }
        MetadataRequestV1::Reject { candidate_id } => MetadataInputV1::Reject {
            input: Box::new(MetadataRejectInputV1 {
                candidate_id: candidate_id.clone(),
                import_result_id: captured
                    .header_rows
                    .as_ref()
                    .and_then(|rows| rows.rows.first())
                    .and_then(|row| optional_text(&row[0])),
            }),
        },
    })
}

fn doi(value: &str) -> PreparedDoi {
    let display = crate::doi::clean_doi(value).unwrap_or_else(|| value.trim().to_owned());
    PreparedDoi {
        normalized: crate::doi::normalized_doi(&display),
        display,
    }
}
