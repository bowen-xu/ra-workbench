//! Original ordinary-chat preparation, shared by static and Native source adapters.
use super::*;
use crate::agent::writing_context::WritingContextReader;

pub(crate) fn materialize_in(
    tx: &Transaction<'_>,
    store: &AppStore,
    request: PrepareRequest,
    model: (
        research_agent_core::protocol::ModelIdentity,
        Option<String>,
        Option<String>,
    ),
) -> Result<Preview, Error> {
    materialize_with_context(tx, store, request, model, None).map_err(|error| match error {
        sources::MaterializationError::Source(error) => error,
        sources::MaterializationError::Writing(_) => Error::SourceUnavailable,
    })
}

/// Build the complete frozen submission inside the caller's transaction.
/// The caller owns commit, rollback, and any operation receipt.
pub(crate) fn materialize_with_context(
    tx: &Transaction<'_>,
    store: &AppStore,
    request: PrepareRequest,
    model: (
        research_agent_core::protocol::ModelIdentity,
        Option<String>,
        Option<String>,
    ),
    mut writing: Option<&mut dyn WritingContextReader>,
) -> Result<Preview, sources::MaterializationError> {
    check_conversation(tx, request.conversation_id)?;
    let mut context = resource_grants::grants_with_scope(
        tx,
        &request.resources,
        crate::agent::writing_resources::context_scope(
            store.writing_resource_source(),
            writing.as_deref(),
        ),
    )?;
    if let Some(retrieval) = request.retrieval.as_ref() {
        let source = crate::agent::retrieval_context::consume_in(
            store
                .retrieval_trace_source()
                .ok_or(Error::SourceUnavailable)?,
            tx,
            retrieval,
            crate::agent::retrieval_context::RetrievalScope::Conversation {
                conversation_id: &request.conversation_id.to_string(),
            },
        )
        .map_err(|_| Error::SourceUnavailable)?;
        context.push(source);
    }
    let queued_user_input = request.user_input.clone();
    let mut user_input = request.user_input;
    references::resolve_with_context(tx, store, &mut user_input, &context, writing.take())?;
    let (skills, skill_instructions) = crate::agent::skills::injection::freeze_application(
        store.agent_configuration_directory(),
        &request.text,
        &user_input,
    )
    .map_err(|_| Error::InvalidInput)?;
    crate::agent::skills::injection::validate_mentions(store, tx, &user_input, &context)
        .map_err(|_| Error::InvalidInput)?;
    let input = SubmissionInput {
        remote_compaction_v2: crate::agent::config::freeze_remote_compaction(
            store.agent_configuration_directory(),
            &model.0,
        )
        .map_err(|_| Error::Unavailable)?,
        text: request.text,
        user_input,
        skill_instructions,
        model: model.0,
        reasoning_effort: model.1,
        service_tier: model.2,
        context,
        instructions: vec![],
        instruction_configs: vec![],
        skills,
    };
    let (record, confirmation) = if let Some(confirmation) = request.queue {
        let (record, state) = crate::agent::repository::input_queue::prepare_resolved_preview_in(
            tx,
            request.conversation_id,
            request.request_id,
            confirmation.expected_revision,
            confirmation.input_id,
            input,
            &queued_user_input,
        )
        .map_err(|_| Error::ConfirmationMismatch)?;
        (
            record,
            Some(crate::agent::queue_contract::Confirmation {
                input_id: confirmation.input_id,
                expected_revision: state.revision(),
            }),
        )
    } else {
        (
            submissions::insert_in(tx, request.conversation_id, request.request_id, input)
                .map_err(|_| Error::InvalidInput)?,
            None,
        )
    };
    let mut preview = Preview::from(record);
    preview.queue = confirmation;
    Ok(preview)
}
