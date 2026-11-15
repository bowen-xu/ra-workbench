use crate::ai::error::AiError;
use research_workbench_native_sdk::{derived_view::*, evidence_matrix::EvidenceMatrixCellDraft};
use rusqlite::{Connection, OptionalExtension, params};
pub(super) fn validate_with<E: From<AiError>>(
    sql: &str,
    db: &Connection,
    draft: &DerivedResearchViewDraft,
    decide: &mut impl FnMut(DerivedValidationInputV1) -> Result<(), E>,
) -> Result<(), E> {
    decide(DerivedValidationInputV1::Header {
        draft: draft.clone(),
    })?;
    let mut anchors = Vec::new();
    for (ordinal, input) in draft.inputs.iter().enumerate() {
        decide(DerivedValidationInputV1::Input {
            ordinal,
            input: input.clone(),
        })?;
        let json = db
            .query_row(
                sql,
                params![input.object_id, input.object_revision, draft.project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(AiError::from)?
            .ok_or_else(|| AiError::InvalidInput("派生视图输入修订不在项目范围内".into()))?;
        let snapshot: EvidenceMatrixCellDraft = serde_json::from_str(&json)
            .map_err(|_| AiError::InvalidInput("矩阵单元版本存储格式无效".into()))?;
        decide(DerivedValidationInputV1::Coverage {
            ordinal,
            snapshot: snapshot.clone(),
        })?;
        anchors.extend(
            snapshot
                .statements
                .into_iter()
                .flat_map(|statement| statement.evidence_anchor_ids),
        );
    }
    for (ordinal, item) in draft.items.iter().enumerate() {
        decide(DerivedValidationInputV1::Item {
            ordinal,
            view_kind: draft.view_kind.clone(),
            item: item.clone(),
            allowed_anchors: anchors.clone(),
        })?;
    }
    Ok(())
}
