//! Fixed original revision reads, interleaved with the selected domain decisions.
use crate::{
    ai::error::AiError,
    runtime_plugins::{RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data},
};
use research_workbench_native_sdk::{
    analysis_workflow::matrix::*,
    generation::{AiMatrixSynthesisContext, AiMatrixSynthesisContextDraft},
};
use rusqlite::{Connection, OptionalExtension, params};

const ASSETS: [(&str, &str); 2] = [
    (
        "matrix-synthesis-matches.v1.sql",
        "4c0ddab235f558219d7d75d4a1f8a33d85781d36fc7cdbe9a46f554d6a7e3f4b",
    ),
    (
        "matrix-synthesis-cell.v1.sql",
        "3c5ade2b1a99c33750bca885f70801a0418a71be384834383b1e6729c5c2be5f",
    ),
];
pub(crate) struct MatrixContextSql([String; 2]);
impl MatrixContextSql {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut statements = Vec::with_capacity(2);
        for (name, hash) in ASSETS {
            statements.push(data::load_text(
                catalog,
                package,
                "literature_library",
                &format!("data/{name}"),
                hash,
            )?);
        }
        Ok(Self(
            statements
                .try_into()
                .expect("two matrix context statements"),
        ))
    }

    pub(crate) fn resolve<E: From<AiError>>(
        &self,
        db: &Connection,
        draft: Option<&AiMatrixSynthesisContextDraft>,
        scope: Vec<(String, String)>,
        mut decide: impl FnMut(MatrixContextInputV1) -> Result<MatrixContextDecisionV1, E>,
    ) -> Result<Option<AiMatrixSynthesisContext>, E> {
        let Some(draft) = draft else {
            return Ok(None);
        };
        validated(decide(MatrixContextInputV1::Header {
            draft: draft.clone(),
        })?)?;
        let matches = db
            .query_row(
                self.0[0].trim_end_matches('\n'),
                params![draft.matrix_id, draft.project_id, draft.matrix_revision],
                |row| row.get(0),
            )
            .map_err(AiError::from)?;
        validated(decide(MatrixContextInputV1::Revision { matches })?)?;
        let mut ids = Vec::new();
        let mut cells = Vec::new();
        for input in &draft.inputs {
            validated(decide(MatrixContextInputV1::Input {
                input: input.clone(),
                prior_cell_ids: ids.clone(),
            })?)?;
            ids.push(input.object_id.clone());
            let (paper_id, json) = db
                .query_row(
                    self.0[1].trim_end_matches('\n'),
                    params![input.object_id, input.object_revision, draft.matrix_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(AiError::from)?
                .ok_or_else(|| {
                    AiError::InvalidInput("多文献综合输入 revision 不属于所选矩阵".into())
                })?;
            let snapshot: research_workbench_native_sdk::evidence_matrix::EvidenceMatrixCellDraft =
                serde_json::from_str(&json)
                    .map_err(|_| AiError::InvalidInput("矩阵单元 revision 无法解码".into()))?;
            validated(decide(MatrixContextInputV1::Coverage {
                snapshot: snapshot.clone(),
            })?)?;
            cells.push(MatrixCellObservationV1 { paper_id, snapshot });
        }
        match decide(MatrixContextInputV1::Finish {
            draft: draft.clone(),
            cells,
            scope,
        })? {
            MatrixContextDecisionV1::Resolved { result } => result
                .map(Some)
                .map_err(|error| E::from(AiError::from(error))),
            _ => Err(E::from(AiError::TemporaryUnavailable)),
        }
    }
}
fn validated<E: From<AiError>>(decision: MatrixContextDecisionV1) -> Result<(), E> {
    match decision {
        MatrixContextDecisionV1::Validated { rejection: None } => Ok(()),
        MatrixContextDecisionV1::Validated {
            rejection: Some(error),
        } => Err(E::from(AiError::from(error))),
        _ => Err(E::from(AiError::TemporaryUnavailable)),
    }
}
