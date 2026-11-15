//! Preserve the original outer preparation order through finite domain decisions.
use super::*;
use crate::{
    ai::{
        analysis_support::{AnalysisResources, PreparedWorkflow},
        error::{AiCommandError, AiError},
        generation_support::resolve_overview_profile_selection,
        paper_analysis_contracts::{
            DoclingPaperOverviewDraft, PaperAnalysisMessage, PaperAnalysisMessageKind,
            PaperAnalysisStartDraft,
        },
        paper_reading_contracts::PrepareRequest,
        reading_contracts::LlmModelSelectionDraft,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::analysis_workflow::PaperAnalysisWorkflowDraft;
mod source;

impl LiteratureTraceService {
    pub(crate) async fn prepare_analysis_workflow(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: PaperAnalysisStartDraft,
    ) -> Result<PreparedWorkflow, AiCommandError> {
        let owner = Arc::clone(self);
        tokio::spawn(async move {
            owner
                .prepare_analysis_workflow_owned(store, key, draft)
                .await
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
    }

    async fn prepare_analysis_workflow_owned(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: PaperAnalysisStartDraft,
    ) -> Result<PreparedWorkflow, AiCommandError> {
        self.validate_analysis_workflow(
            &key,
            WorkflowInputV1::ValidateStart {
                revision: draft.overview_profile_selection_revision,
                has_template: draft.paper_template_ref.is_some(),
            },
        )
        .await?;
        let template_profile = match &draft.paper_template_ref {
            Some(reference) => Some(
                self.resolve_analysis_template(store.clone(), &key, &draft.paper_id, reference)
                    .await?,
            ),
            None => None,
        };
        let profile = if let Some(profile) = &template_profile {
            profile.clone()
        } else {
            let source = store.clone();
            let paper = draft.paper_id.clone();
            let revision = draft.overview_profile_selection_revision;
            tokio::task::spawn_blocking(move || {
                let db = source.connection()?;
                resolve_overview_profile_selection(&db, &paper, revision)
            })
            .await
            .map_err(|_| AiError::TemporaryUnavailable)??
        };
        let (_, snapshot) = self
            .prepare_analysis_context(
                store.clone(),
                key.clone(),
                PrepareRequest {
                    paper_id: draft.paper_id.clone(),
                    attachment_ids: draft.attachment_ids.clone(),
                    image_assets: draft.image_assets.clone(),
                },
            )
            .await
            .map_err(AiCommandError::from_analysis_workflow)?;
        let template_json = template_profile
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| AiError::TemporaryUnavailable)?;
        self.validate_analysis_reuse(
            store.clone(),
            &key,
            &draft,
            &snapshot.snapshot_id,
            template_json,
        )
        .await?;
        let source = store.clone();
        let selected = draft.model_selection.clone();
        let selection = tokio::task::spawn_blocking(move || {
            let db = source.connection()?;
            AnalysisResources::new(&source)
                .selection(&db, "paper_overview", selected.as_ref())
                .map_err(|error| error.into_ai_error())
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)??;
        let workflow = self
            .create_analysis_workflow(
                store.clone(),
                key.clone(),
                PaperAnalysisWorkflowDraft {
                    paper_id: draft.paper_id.clone(),
                    context_snapshot_id: snapshot.snapshot_id.clone(),
                    provider_id: selection.provider_id.clone(),
                    model_id: selection.model_id.clone(),
                    reasoning_effort: selection.reasoning_effort.clone(),
                    prompt_set_version: "paper-analysis-tools-v3".into(),
                },
            )
            .await
            .map_err(AiCommandError::from_analysis_workflow)?;
        let (decision, origin) = self
            .analysis_workflow_decision(
                &key,
                WorkflowInputV1::OverviewMessages {
                    workflow: Box::new(workflow.clone()),
                },
            )
            .await
            .map_err(template::technical)?;
        let WorkflowDecisionV1::Messages { mut messages } = decision else {
            return Err(template::identified(
                "plugin_host_response_mismatch",
                "分析工作流规范消息种类不匹配。",
                &origin,
            )
            .into());
        };
        messages.push(PaperAnalysisMessage::new(
            PaperAnalysisMessageKind::TaskPrompt,
            serde_json::to_string(&profile).map_err(|_| AiError::TemporaryUnavailable)?,
        ));
        let source = store.clone();
        let ids = draft.skill_source_ids;
        let inputs = draft.resource_inputs;
        let resources = tokio::task::spawn_blocking(move || {
            AnalysisResources::new(&source)
                .freeze_application_resources(&ids, &inputs)
                .map_err(|error| error.into_ai_error())
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)??;
        let workflow = self
            .start_analysis_attempt(
                store,
                key,
                StartAnalysisAttempt {
                    workflow_id: workflow.id,
                    turn_kind: "paper_overview".into(),
                    messages,
                    resources,
                },
            )
            .await?;
        let attempt = workflow
            .turns
            .iter()
            .find(|turn| turn.kind == "paper_overview")
            .and_then(|turn| turn.attempts.last())
            .ok_or(AiError::TemporaryUnavailable)?;
        let overview_draft = DoclingPaperOverviewDraft {
            paper_id: draft.paper_id,
            attachment_ids: draft.attachment_ids,
            context_snapshot_id: Some(snapshot.snapshot_id),
            overview_profile_selection_revision: draft.overview_profile_selection_revision,
            paper_template_ref: draft.paper_template_ref,
            model_selection: Some(LlmModelSelectionDraft {
                provider_id: selection.provider_id,
                model_id: selection.model_id,
                reasoning_effort: selection.reasoning_effort,
            }),
            image_assets: vec![],
            shared_conversation_id: workflow.conversation_id,
            shared_request_id: attempt.shared_request_id,
        };
        Ok(PreparedWorkflow {
            workflow,
            overview_draft,
        })
    }

    async fn validate_analysis_workflow(
        &self,
        key: &str,
        input: WorkflowInputV1,
    ) -> Result<(), AiError> {
        let (decision, origin) = self
            .analysis_workflow_decision(key, input)
            .await
            .map_err(template::technical)?;
        match decision {
            WorkflowDecisionV1::Validated { rejection: None } => Ok(()),
            WorkflowDecisionV1::Validated {
                rejection: Some(error),
            } => Err(template::rejected(error, &origin)),
            _ => Err(template::identified(
                "plugin_host_response_mismatch",
                "分析工作流校验种类不匹配。",
                &origin,
            )),
        }
    }
}
