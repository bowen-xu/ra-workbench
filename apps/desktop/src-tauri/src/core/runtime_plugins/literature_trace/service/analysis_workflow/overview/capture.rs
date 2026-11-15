//! Fixed Core reads in the original preview order, reusing request-bound decisions.
use super::*;
use crate::ai::{
    generation_support as run_repository, task_evidence_support::TaskEvidenceFacts,
    template_facts::TemplateFacts,
};
use research_workbench_native_sdk::paper_context_wire::encode_payload;
use rusqlite::Connection;
mod matrix;

struct Observation {
    input: WorkflowInputV1,
    decision: WorkflowDecisionV1,
    origin: DecisionOrigin,
}
#[derive(Default)]
pub(super) struct OverviewPolicy {
    observations: Vec<Observation>,
    template: template::TemplateResolution,
    pub(super) matrix_sql:
        Option<crate::runtime_plugins::literature_matrix::context::MatrixContextSql>,
}
pub(super) enum CaptureError {
    Original(Box<AiError>),
    Need(Box<WorkflowInputV1>),
    MatrixResources,
}
impl From<AiError> for CaptureError {
    fn from(error: AiError) -> Self {
        Self::Original(Box::new(error))
    }
}

impl OverviewPolicy {
    pub(super) fn latest_origin(&self) -> Option<&DecisionOrigin> {
        self.observations.last().map(|value| &value.origin)
    }

    pub(super) fn decision(
        &self,
        index: usize,
        input: WorkflowInputV1,
    ) -> Result<(&WorkflowDecisionV1, &DecisionOrigin), CaptureError> {
        self.observed(index, input)
            .map(|value| (&value.decision, &value.origin))
    }
    pub(super) fn incomplete_error(&self, key: &str) -> AiError {
        match self.observations.last() {
            Some(observed) => template::identified(
                "plugin_host_response_mismatch",
                "结构化阅读运行准备缺少原有限决定。",
                &observed.origin,
            ),
            None => template::technical(
                LiteratureTraceService::error(
                    "plugin_host_response_mismatch",
                    "结构化阅读运行准备缺少原有限决定。",
                )
                .package(key),
            ),
        }
    }
    pub(super) fn record(
        &mut self,
        input: WorkflowInputV1,
        decision: WorkflowDecisionV1,
        origin: DecisionOrigin,
    ) {
        match (input, decision) {
            (WorkflowInputV1::Template { input }, WorkflowDecisionV1::Template { decision }) => {
                self.template.record(*input, decision, origin)
            }
            (input, decision) => self.observations.push(Observation {
                input,
                decision,
                origin,
            }),
        }
    }

    pub(super) fn capture(
        &self,
        db: &Connection,
        credentials: &JsonCredentialStore,
        draft: &AiTaskSpecDraft,
    ) -> Result<AiTaskPreview, CaptureError> {
        if !matches!(
            draft.task_kind.as_str(),
            "paper_overview"
                | "paper_profile_classification"
                | "paper_suggestions"
                | "reference_extraction"
                | "matrix_synthesis"
        ) || (draft.task_kind == "paper_overview"
            && (draft.base_candidate_id.is_some() || draft.matrix_synthesis_context.is_some()))
        {
            return Err(AiError::InvalidInput("结构化阅读运行预览参数无效".into()).into());
        }
        run_repository::validate_task_before_policy(draft)?;
        let encoded = encode_payload(draft).map_err(|_| AiError::TemporaryUnavailable)?;
        self.validated(
            0,
            WorkflowInputV1::TaskShape {
                draft_cbor_base64: encoded,
            },
        )?;
        run_repository::validate_overview_retry_shape(draft)?;
        let scope = run_repository::resolve_task_scope(db, credentials, draft)?;
        let suggestion = matches!(
            draft.task_kind.as_str(),
            "paper_suggestions" | "reference_extraction" | "matrix_synthesis"
        );
        if suggestion {
            self.validated(
                1,
                WorkflowInputV1::TaskScope {
                    task_kind: draft.task_kind.clone(),
                    item_kinds: scope
                        .items
                        .iter()
                        .map(|item| item.item_kind.clone())
                        .collect(),
                },
            )?;
        } else {
            run_repository::validate_fixed_task_scope(&draft.task_kind, &scope.items)?;
        }
        if draft.task_kind == "paper_profile_classification" {
            let hash = run_repository::scope_hash(draft, &scope.items, None, None, None)?;
            return Ok(scope.into_preview(draft, hash, None, None, Vec::new()));
        }
        if draft.task_kind == "matrix_synthesis" {
            let (context, evidence) = self.matrix_preview(db, draft, &scope.items)?;
            let hash = run_repository::scope_hash(draft, &scope.items, None, None, Some(&context))?;
            return Ok(scope.into_preview(draft, hash, None, Some(context), evidence));
        }
        let item = &scope.items[0];
        if draft.task_kind == "reference_extraction" {
            let hash = run_repository::scope_hash(draft, &scope.items, None, None, None)?;
            let input = WorkflowInputV1::SelectionEvidence {
                source: TaskEvidenceFacts::new(db).anchor(&item.object_id)?,
            };
            let observed = self.observed(2, input)?;
            let WorkflowDecisionV1::SelectionEvidence { result } = &observed.decision else {
                return Err(template::identified(
                    "plugin_host_response_mismatch",
                    "选区证据结果种类不匹配。",
                    &observed.origin,
                )
                .into());
            };
            let evidence = result
                .clone()
                .map_err(|error| template::rejected(error, &observed.origin))?;
            return Ok(scope.into_preview(draft, hash, None, None, evidence));
        }
        let profile = if let Some(reference) = &draft.paper_template_ref {
            let paper = item
                .parent_object_id
                .as_deref()
                .ok_or_else(|| AiError::InvalidInput("模板提取附件缺少所属文献".into()))?;
            match self
                .template
                .capture(&TemplateFacts::new(db), paper, reference)?
            {
                template::CapturedTemplate::Need(input) => {
                    return Err(CaptureError::Need(Box::new(WorkflowInputV1::Template {
                        input: Box::new(input),
                    })));
                }
                template::CapturedTemplate::Resolved(profile) => Some(profile),
            }
        } else if let Some(revision) = draft.overview_profile_selection_revision {
            let paper = item
                .parent_object_id
                .as_deref()
                .ok_or_else(|| AiError::InvalidInput("概要附件缺少所属文献".into()))?;
            Some(run_repository::resolve_overview_profile_selection(
                db, paper, revision,
            )?)
        } else {
            None
        };
        // The canonical new-overview constructor has no retry or matrix context.
        // Both original optional policies return before any read in this case.
        let hash = run_repository::scope_hash(draft, &scope.items, None, profile.as_ref(), None)?;
        let facts = TaskEvidenceFacts::new(db);
        let attachment = facts.attachment(&item.object_id)?;
        self.validated(
            if suggestion { 2 } else { 1 },
            WorkflowInputV1::PageAttachment {
                attachment: attachment.clone(),
            },
        )?;
        let attachment = attachment.ok_or(AiError::TemporaryUnavailable)?;
        let input = WorkflowInputV1::PagePreview {
            attachment_id: item.object_id.clone(),
            attachment,
            pages: facts.page_texts(&item.object_id)?,
        };
        let observed = self.observed(if suggestion { 3 } else { 2 }, input)?;
        let WorkflowDecisionV1::PagePreview { result } = &observed.decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "运行预览页证据结果不匹配。",
                &observed.origin,
            )
            .into());
        };
        let evidence = result
            .clone()
            .map_err(|error| template::rejected(error, &observed.origin))?;
        Ok(scope.into_preview(draft, hash, profile, None, evidence))
    }

    fn observed(&self, index: usize, input: WorkflowInputV1) -> Result<&Observation, CaptureError> {
        let Some(observed) = self.observations.get(index) else {
            return Err(CaptureError::Need(Box::new(input)));
        };
        if observed.input != input {
            return Err(template::identified(
                "plugin_host_source_changed",
                "结构化阅读运行来源已变化，请重新准备。",
                &observed.origin,
            )
            .into());
        }
        Ok(observed)
    }
    pub(super) fn validated(
        &self,
        index: usize,
        input: WorkflowInputV1,
    ) -> Result<(), CaptureError> {
        let observed = self.observed(index, input)?;
        match &observed.decision {
            WorkflowDecisionV1::Validated { rejection: None } => Ok(()),
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => Err(template::rejected(error.clone(), &observed.origin).into()),
            _ => Err(template::identified(
                "plugin_host_response_mismatch",
                "运行预览校验结果不匹配。",
                &observed.origin,
            )
            .into()),
        }
    }
}
