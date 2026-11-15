//! One source materializer for ordinary and queued compact preparation.
use super::*;
use crate::agent::retrieval_context::{RetrievalScope, consume_in_sql};
use crate::agent::writing_context::ContextNeed;
use crate::{
    agent::writing_context::WritingContextReader, ai::structured_resource_validation::Flow,
};
use sha2::Digest;

pub(crate) type PreparationFlow<T> = Result<T, Flow<WritingSubmissionError, ContextNeed>>;
impl From<WritingSubmissionError> for Flow<WritingSubmissionError, ContextNeed> {
    fn from(error: WritingSubmissionError) -> Self {
        Self::original(error)
    }
}
impl From<SubmissionError> for Flow<WritingSubmissionError, ContextNeed> {
    fn from(error: SubmissionError) -> Self {
        Self::original(error.into())
    }
}
impl From<WritingContextError> for Flow<WritingSubmissionError, ContextNeed> {
    fn from(error: WritingContextError) -> Self {
        Self::original(error.into())
    }
}
impl From<crate::agent::instructions::InstructionReadError>
    for Flow<WritingSubmissionError, ContextNeed>
{
    fn from(error: crate::agent::instructions::InstructionReadError) -> Self {
        Self::original(error.into())
    }
}

#[derive(Clone)]
pub(crate) struct FrozenWritingModel {
    pub identity: research_agent_core::protocol::ModelIdentity,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
}

/// The caller owns the transaction which freezes input, purpose and queue claim.
/// No model, tool execution or independently committed preparation is created.
#[cfg(test)]
pub(crate) fn materialize_submission_in(
    tx: &rusqlite::Transaction<'_>,
    store: &AppStore,
    request: WritingSubmissionRequest,
    model: FrozenWritingModel,
) -> Result<SubmissionInput, WritingSubmissionError> {
    materialize_submission_with_context(tx, store, request, model, None).map_err(
        |flow| match flow {
            Flow::Fail(failure) => failure.error,
            Flow::Need(_) => WritingSubmissionError::Unavailable,
        },
    )
}

pub(crate) fn materialize_submission_with_context(
    tx: &rusqlite::Transaction<'_>,
    store: &AppStore,
    request: WritingSubmissionRequest,
    model: FrozenWritingModel,
    mut writing: Option<&mut dyn WritingContextReader>,
) -> PreparationFlow<SubmissionInput> {
    let manuscript = request.context.manuscript_id;
    check_binding(tx, request.conversation_id, manuscript)?;
    let FrozenWritingModel {
        identity,
        reasoning_effort,
        service_tier,
    } = model;
    let selected_project = request.resources.iter().find(|resource| matches!(resource,
    crate::agent::context_session::sources::Resource::Manuscript {resource_id,..} if resource_id == &manuscript.to_string()
));
    let mut writing_context = request.context;
    if let Some(resource) = selected_project {
        let parts = crate::agent::context_session::group_scope::parts_with_scope(
            tx,
            resource,
            crate::agent::writing_resources::context_scope(
                store.writing_resource_source(),
                writing.as_deref(),
            ),
        )
        .map_err(scope_error)?;
        writing_context.files.retain(|file| parts.iter().any(|part| part.selected && matches!(&part.resource,crate::agent::context_session::sources::Resource::ManuscriptFile {path,..} if path == &file.relative_path)));
    }
    let mut context = if writing_context.files.is_empty() {
        vec![]
    } else {
        match writing.as_mut() {
            Some(reader) => reader
                .selected(tx, store.manuscript_directory(), writing_context)
                .map_err(|flow| flow.map_error(WritingSubmissionError::Context))?,
            None => store.materialize_writing_context_in(tx, writing_context)?,
        }
    };
    // Only the editable directory determines workspace instructions and trust.
    let mut workspace_context = context.clone();
    let mut extra = Vec::new();
    for resource in &request.resources {
        let rows = if matches!(resource,crate::agent::context_session::sources::Resource::Manuscript {resource_id,..} if resource_id == &manuscript.to_string())
            || matches!(resource,crate::agent::context_session::sources::Resource::ManuscriptFile {resource_id,..} if resource_id == &manuscript)
        {
            match writing.as_mut() {
                Some(reader) => crate::agent::context_session::sources::materialize_with_context(
                    tx,
                    store,
                    std::slice::from_ref(resource),
                    Some(&mut **reader),
                ),
                None => crate::agent::context_session::sources::materialize_with_context(
                    tx,
                    store,
                    std::slice::from_ref(resource),
                    None,
                ),
            }
        } else {
            crate::agent::context_session::resource_grants::grants_with_scope(
                tx,
                std::slice::from_ref(resource),
                crate::agent::writing_resources::context_scope(
                    store.writing_resource_source(),
                    writing.as_deref(),
                ),
            )
            .map_err(crate::agent::context_session::sources::MaterializationError::Source)
        };
        let mut rows = match rows {
            Ok(rows) => rows,
            Err(crate::agent::context_session::sources::MaterializationError::Source(
                crate::agent::context_session::Error::SourceUnavailable,
            )) => continue,
            Err(crate::agent::context_session::sources::MaterializationError::Source(
                crate::agent::context_session::Error::NativeCatalogUnavailable(error),
            )) => return Err(WritingSubmissionError::from(error).into()),
            Err(crate::agent::context_session::sources::MaterializationError::Writing(
                Flow::Need(input),
            )) => return Err(Flow::Need(input)),
            Err(crate::agent::context_session::sources::MaterializationError::Writing(
                Flow::Fail(_),
            )) => continue,
            Err(_) => return Err(SubmissionError::InvalidInput.into()),
        };
        extra.append(&mut rows);
    }
    for source in &mut extra {
        let mut anchor = source
            .anchor
            .as_deref()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        anchor["supplementalContext"] = serde_json::json!(
            !(selected_project.is_some()
                && anchor["contextOwner"]["kind"] == "manuscript"
                && anchor["contextOwner"]["resourceId"] == manuscript.to_string())
        );
        source.anchor = Some(anchor.to_string());
    }
    for source in extra {
        if let Some(existing) = context
            .iter_mut()
            .find(|existing| existing.source_id == source.source_id)
        {
            // Preserve the live buffer, but apply the explicit child permission to it.
            if selected_project.is_some() {
                let mut anchor: serde_json::Value = serde_json::from_str(
                    existing.anchor.as_deref().ok_or(SubmissionError::Corrupt)?,
                )
                .map_err(|_| SubmissionError::Corrupt)?;
                let access: serde_json::Value =
                    serde_json::from_str(source.anchor.as_deref().ok_or(SubmissionError::Corrupt)?)
                        .map_err(|_| SubmissionError::Corrupt)?;
                for key in ["contextAccess", "contextOwner", "supplementalContext"] {
                    anchor[key] = access[key].clone();
                }
                existing.anchor = Some(anchor.to_string());
            }
        } else {
            context.push(source);
        }
    }
    if selected_project.is_some() {
        workspace_context = context
            .iter()
            .filter(|source| {
                source
                    .anchor
                    .as_deref()
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                    .is_some_and(|anchor| {
                        anchor["manuscriptId"] == manuscript.to_string()
                            && anchor["supplementalContext"] != true
                    })
            })
            .cloned()
            .collect();
    }
    if let Some(resource) = selected_project
        .filter(|resource| crate::agent::context_session::group_scope::is_dynamic(resource))
    {
        let mut grants = crate::agent::context_session::resource_grants::grants_with_scope(
            tx,
            std::slice::from_ref(resource),
            crate::agent::writing_resources::context_scope(
                store.writing_resource_source(),
                writing.as_deref(),
            ),
        )
        .map_err(scope_error)?;
        for grant in &mut grants {
            let mut anchor: serde_json::Value =
                serde_json::from_str(grant.anchor.as_deref().ok_or(SubmissionError::Corrupt)?)
                    .map_err(|_| SubmissionError::Corrupt)?;
            anchor["supplementalContext"] = serde_json::json!(false);
            anchor["contextOwner"] =
                serde_json::json!({"kind":"manuscript","resourceId":manuscript});
            anchor["contextScope"] = serde_json::to_value(
                crate::agent::context_session::group_scope::compact(resource),
            )
            .map_err(|_| SubmissionError::InvalidInput)?;
            grant.anchor = Some(anchor.to_string());
        }
        context.extend(grants);
    }
    let mut user_input = request.user_input;
    let references = match writing.as_mut() {
        Some(reader) => crate::agent::context_session::references::resolve_with_context(
            tx,
            store,
            &mut user_input,
            &context,
            Some(&mut **reader),
        ),
        None => crate::agent::context_session::references::resolve_with_context(
            tx,
            store,
            &mut user_input,
            &context,
            None,
        ),
    };
    match references {
        Ok(()) => {}
        Err(crate::agent::context_session::sources::MaterializationError::Source(
            crate::agent::context_session::Error::NativeCatalogUnavailable(error),
        )) => return Err(WritingSubmissionError::from(error).into()),
        Err(crate::agent::context_session::sources::MaterializationError::Source(_)) => {
            return Err(SubmissionError::InvalidInput.into());
        }
        Err(crate::agent::context_session::sources::MaterializationError::Writing(flow)) => {
            return Err(flow.map_error(WritingSubmissionError::from));
        }
    }
    if let Some(retrieval) = request.retrieval.as_ref() {
        context.push(
            consume_in_sql(
                store
                    .retrieval_trace_source()
                    .ok_or(SubmissionError::Conflict)?,
                tx,
                retrieval,
                RetrievalScope::Manuscript {
                    manuscript_id: &manuscript.to_string(),
                },
            )
            .map_err(|error| {
                use crate::agent::repository::SqlRepositoryError as E;
                match error {
                    E::Sql(cause) => WritingSubmissionError::Sql(cause),
                    E::Domain(crate::agent::retrieval_context::RetrievalContextError::Invalid) => {
                        SubmissionError::InvalidInput.into()
                    }
                    E::Domain(crate::agent::retrieval_context::RetrievalContextError::Conflict) => {
                        SubmissionError::Conflict.into()
                    }
                }
            })?,
        );
    }
    research_agent_core::context::assemble_context_text(
        research_agent_core::context::ContextIdentity::new(
            "writing",
            manuscript.to_string(),
            format!(
                "sha256:{:x}",
                sha2::Sha256::digest(
                    serde_json::to_vec(&context)
                        .map_err(|_| WritingSubmissionError::Unavailable)?
                )
            ),
        )
        .map_err(|_| WritingSubmissionError::Unavailable)?,
        request.request_id.to_string(),
        &context,
    )
    .map_err(|_| WritingSubmissionError::Unavailable)?;
    let crate::agent::writing_instructions::TrustedContext {
        instructions,
        instruction_configs,
        mut skills,
    } = crate::agent::writing_instructions::materialize_in(
        tx,
        crate::agent::writing_instructions::InstructionEnvironment {
            manuscripts: store.manuscript_directory(),
            user_configuration: store.agent_configuration_directory(),
            source_directory: store.writing_source_directory(),
        },
        manuscript,
        &workspace_context,
        &request.text,
        [request.session_config, request.turn_config],
    )?;
    let (application_skills, mut skill_instructions) =
        crate::agent::skills::injection::freeze_application(
            store.agent_configuration_directory(),
            &request.text,
            &user_input,
        )?;
    let workspace_items: Vec<_> = user_input.iter().filter(|item| matches!(item, research_agent_core::user_input::UserInput::Skill { identity, .. } if identity.environment_id != "application")).cloned().collect();
    skills = research_agent_core::skills::selection::select_inputs(
        skills.into_iter().map(|skill| skill.descriptor).collect(),
        &request.text,
        &workspace_items,
    )
    .map_err(|_| SubmissionError::InvalidInput)?;
    if skills.iter().any(|skill| skill.explicit) {
        let root = store
            .manuscript_directory()
            .canonicalize()
            .map_err(|_| WritingSubmissionError::Unavailable)?
            .join(manuscript.to_string())
            .join("source");
        skill_instructions.extend(crate::agent::skills::injection::load_prompts(
            &root,
            &skills,
            |identity| {
                crate::agent::repository::instruction_trust::is_trusted(tx, manuscript, identity)
                    .map_err(|_| crate::agent::instructions::InstructionReadError::Unavailable)?
                    .then_some(())
                    .ok_or(crate::agent::instructions::InstructionReadError::Unauthorized)
            },
        )?);
    }
    skills.extend(application_skills);
    crate::agent::skills::injection::validate_mentions(store, tx, &user_input, &context)?;

    Ok(SubmissionInput {
        remote_compaction_v2: crate::agent::config::freeze_remote_compaction(
            store.agent_configuration_directory(),
            &identity,
        )
        .map_err(|_| WritingSubmissionError::Unavailable)?,
        user_input,
        skill_instructions,
        instructions,
        instruction_configs,
        skills,
        text: request.text,
        model: identity,
        reasoning_effort,
        service_tier,
        context,
    })
}

fn scope_error(error: crate::agent::context_session::Error) -> WritingSubmissionError {
    match error {
        crate::agent::context_session::Error::NativeCatalogUnavailable(error) => error.into(),
        _ => SubmissionError::InvalidInput.into(),
    }
}
