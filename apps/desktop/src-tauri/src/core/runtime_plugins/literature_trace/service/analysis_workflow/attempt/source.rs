//! Original shared attempt input facts. Native waits receive owned values only.
use crate::{
    ai::{
        analysis_support::{AnalysisJournal, AnalysisSupportError, FrozenResources},
        error::AiError,
        paper_analysis_contracts::{PaperAnalysisMessage, PaperAnalysisWorkflow},
        paper_reading_contracts::PaperContextSnapshot,
        structured_generation_support::{self as structured, StructuredGenerationFacts},
        structured_resource_validation::FailureCause,
    },
    runtime_plugins::literature_context::{PaperContextResources, SnapshotWitness},
    storage::AppStore,
};
use research_agent_core::{
    protocol::ModelIdentity,
    submission::{ContextText, SubmissionInput},
    user_input::UserInput,
};
use research_workbench_native_sdk::analysis_workflow::prompt::{
    PaperTaskPrompt, PromptSourceV1, TaskPromptInputV1,
};
use rusqlite::Connection;
use uuid::Uuid;

#[derive(Clone, PartialEq)]
pub(super) struct SharedFacts {
    pub snapshot: PaperContextSnapshot,
    witness: SnapshotWitness,
    pub grants: Vec<ContextText>,
    pub prompt: Option<TaskPromptInputV1>,
}

pub(super) fn capture(
    db: &Connection,
    store: &AppStore,
    context: Option<&PaperContextResources>,
    workflow: &PaperAnalysisWorkflow,
    messages: &[PaperAnalysisMessage],
) -> Result<Option<SharedFacts>, AiError> {
    let Some(conversation) = workflow.conversation_id else {
        return Ok(None);
    };
    let context = context.ok_or(AiError::CapabilityUnavailable("literature.paper-context"))?;
    let (snapshot, witness) = context
        .read_snapshot(db, &workflow.context_snapshot_id)
        .map_err(|failure| match failure.failure.cause {
            FailureCause::SourceData(error) => AiError::PluginDatabase(error),
            _ => failure.failure.error,
        })?;
    let attachments = snapshot
        .clone()
        .into_reading_result()
        .attachment_statuses
        .into_iter()
        .map(|attachment| attachment.attachment_id)
        .collect::<Vec<_>>();
    let journal = AnalysisJournal::new(db);
    let grants = journal
        .grants(
            &workflow.paper_id,
            &attachments,
            conversation,
            "structuredReading",
            &workflow.context_snapshot_id,
            store.writing_resource_source(),
        )
        .map_err(AnalysisSupportError::into_ai_error)?;
    let managed = journal
        .managed_templates()
        .map_err(AnalysisSupportError::into_ai_error)?;
    let prompt = if managed {
        let profile = messages
            .iter()
            .find_map(|message| serde_json::from_str(&message.content).ok());
        let facts = StructuredGenerationFacts::new(db, store);
        let target = facts.target(&workflow.paper_id, "structuredReading")?;
        structured::require_writable_target(&grants, &target.id)?;
        let (task_template, reading_contract) = facts.prompt_templates()?;
        Some(TaskPromptInputV1 {
            task_template,
            reading_contract,
            sources: grants
                .iter()
                .filter_map(|source| {
                    structured::prompt_resource(source).map(|resource| PromptSourceV1 {
                        source: source.clone(),
                        resource,
                    })
                })
                .collect(),
            kind: "structuredReading".into(),
            profile,
            preferences: String::new(),
        })
    } else {
        None
    };
    Ok(Some(SharedFacts {
        snapshot,
        witness,
        grants,
        prompt,
    }))
}

pub(super) fn submission(
    workflow: &PaperAnalysisWorkflow,
    attempt: &str,
    facts: &SharedFacts,
    context: Vec<ContextText>,
    prompt: PaperTaskPrompt,
    resources: FrozenResources,
) -> Result<(Uuid, Uuid, SubmissionInput), AiError> {
    let (skills, skill_instructions, resource_inputs) = resources;
    let mut user_input = prompt.user_input;
    user_input.extend(if resource_inputs.is_empty() {
        skills
            .iter()
            .filter(|skill| skill.explicit)
            .map(|skill| UserInput::Skill {
                name: skill.descriptor.name.clone(),
                identity: skill.descriptor.identity.clone(),
            })
            .collect()
    } else {
        resource_inputs
    });
    let request = Uuid::parse_str(attempt).map_err(|_| AiError::TemporaryUnavailable)?;
    research_agent_core::context::assemble_context_text(
        research_agent_core::context::ContextIdentity::new(
            "review",
            workflow.paper_id.clone(),
            facts.snapshot.context_hash.clone(),
        )
        .map_err(|_| AiError::TemporaryUnavailable)?,
        request.to_string(),
        &context,
    )
    .map_err(|_| AiError::TemporaryUnavailable)?;
    let conversation = workflow
        .conversation_id
        .ok_or(AiError::TemporaryUnavailable)?;
    Ok((
        conversation,
        request,
        SubmissionInput {
            remote_compaction_v2: None,
            text: prompt.text,
            user_input,
            skill_instructions,
            model: ModelIdentity {
                provider_id: workflow.provider_id.clone(),
                model_id: workflow.model_id.clone(),
                revision: None,
            },
            reasoning_effort: workflow.reasoning_effort.clone(),
            service_tier: None,
            context,
            instructions: vec![],
            instruction_configs: vec![],
            skills,
        },
    ))
}
