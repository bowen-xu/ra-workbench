//! Original matrix reads are replayed before preview or admission becomes authoritative.
use super::*;
use crate::ai::task_evidence_support::{AiEvidencePreview, AiMatrixSynthesisContext, AiScopeItem};
use research_workbench_native_sdk::analysis_workflow::matrix::*;

impl OverviewPolicy {
    pub(super) fn matrix_preview(
        &self,
        db: &Connection,
        draft: &AiTaskSpecDraft,
        scope: &[AiScopeItem],
    ) -> Result<(AiMatrixSynthesisContext, Vec<AiEvidencePreview>), CaptureError> {
        let sql = self
            .matrix_sql
            .as_ref()
            .ok_or(CaptureError::MatrixResources)?;
        let mut cursor = 2;
        let context = sql
            .resolve(
                db,
                draft.matrix_synthesis_context.as_ref(),
                scope
                    .iter()
                    .map(|item| (item.item_kind.clone(), item.object_id.clone()))
                    .collect(),
                |input| {
                    let (decision, origin) = self.decision(
                        cursor,
                        WorkflowInputV1::MatrixContext {
                            input: Box::new(input),
                        },
                    )?;
                    cursor += 1;
                    let WorkflowDecisionV1::MatrixContext { decision } = decision else {
                        return Err(template::identified(
                            "plugin_host_response_mismatch",
                            "矩阵上下文结果种类不匹配。",
                            origin,
                        )
                        .into());
                    };
                    let error = match decision {
                        MatrixContextDecisionV1::Validated { rejection } => rejection.as_ref(),
                        MatrixContextDecisionV1::Resolved { result } => result.as_ref().err(),
                    };
                    if let Some(error) = error {
                        return Err(template::rejected(error.clone(), origin).into());
                    }
                    Ok::<_, CaptureError>(decision.clone())
                },
            )?
            .ok_or(AiError::TemporaryUnavailable)?;
        let (_, preview) = self.matrix_sources(db, &context.evidence_anchor_ids, &mut cursor)?;
        Ok((context, preview))
    }
    pub(in super::super) fn matrix_evidence(
        &self,
        cursor: &mut usize,
        input: MatrixEvidenceInputV1,
    ) -> Result<MatrixEvidenceDecisionV1, CaptureError> {
        let (decision, origin) = self.decision(
            *cursor,
            WorkflowInputV1::MatrixEvidence {
                input: Box::new(input),
            },
        )?;
        *cursor += 1;
        let WorkflowDecisionV1::MatrixEvidence { decision } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "矩阵证据结果种类不匹配。",
                origin,
            )
            .into());
        };
        let error = match decision {
            MatrixEvidenceDecisionV1::Validated { rejection } => rejection.as_ref(),
            MatrixEvidenceDecisionV1::Preview { result } => result.as_ref().err(),
        };
        if let Some(error) = error {
            return Err(template::rejected(error.clone(), origin).into());
        }
        Ok(decision.clone())
    }
    pub(in super::super) fn matrix_sources(
        &self,
        db: &Connection,
        ids: &[String],
        cursor: &mut usize,
    ) -> Result<
        (
            Vec<crate::ai::task_evidence_support::AnchorEvidence>,
            Vec<AiEvidencePreview>,
        ),
        CaptureError,
    > {
        let facts = TaskEvidenceFacts::new(db);
        let mut sources = Vec::new();
        for id in ids {
            let source = facts.anchor(id)?;
            self.matrix_evidence(
                cursor,
                MatrixEvidenceInputV1::Anchor {
                    source: source.clone(),
                },
            )?;
            sources.push(source.ok_or(AiError::TemporaryUnavailable)?);
        }
        let decision = self.matrix_evidence(
            cursor,
            MatrixEvidenceInputV1::Sources {
                sources: sources.clone(),
            },
        )?;
        let MatrixEvidenceDecisionV1::Preview { result } = decision else {
            return Err(AiError::TemporaryUnavailable.into());
        };
        Ok((sources, result.map_err(AiError::from)?))
    }
}
