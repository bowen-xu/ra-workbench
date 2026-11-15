//! Original ordered materialization inside the Core-owned paper SQL transaction.
use super::*;
use crate::{
    agent::{
        reading_context::{PaperContextReader, ReadingNeed},
        repository::submissions::insert_in_with_images,
    },
    ai::structured_resource_validation::Flow,
};
/// Freeze the complete paper submission inside the caller-owned transaction.
#[cfg(test)]
pub(crate) fn materialize_in(
    tx: &rusqlite::Transaction<'_>,
    store: &AppStore,
    request: PrepareRequest,
    model: (
        research_agent_core::protocol::ModelIdentity,
        Option<String>,
        Option<String>,
    ),
) -> Result<SubmissionRecord, Error> {
    check_binding(tx, request.conversation_id, &request.paper_id)?;
    let mut source = store
        .require_paper_reading_source()
        .map_err(|_| Error::Unavailable)?;
    materialize_after_binding(tx, store, request, model, &mut source).map_err(|flow| match flow {
        Flow::Fail(failure) => failure.error,
        // Function-value sources cannot suspend for a Native decision.
        Flow::Need(_) => unreachable!("static paper materialization cannot pause"),
    })
}

pub(crate) fn materialize_with_reader(
    tx: &rusqlite::Transaction<'_>,
    store: &AppStore,
    request: PrepareRequest,
    model: (
        research_agent_core::protocol::ModelIdentity,
        Option<String>,
        Option<String>,
    ),
    reader: &mut dyn PaperContextReader,
) -> Result<SubmissionRecord, Flow<Error, ReadingNeed>> {
    check_binding(tx, request.conversation_id, &request.paper_id)?;
    materialize_after_binding(tx, store, request, model, reader)
}

fn materialize_after_binding(
    tx: &rusqlite::Transaction<'_>,
    store: &AppStore,
    request: PrepareRequest,
    model: (
        research_agent_core::protocol::ModelIdentity,
        Option<String>,
        Option<String>,
    ),
    reader: &mut dyn PaperContextReader,
) -> Result<SubmissionRecord, Flow<Error, ReadingNeed>> {
    let (identity, reasoning_effort, service_tier) = model;
    let snapshot_request = agent_preview::Request {
        paper_id: request.paper_id.clone(),
        snapshot_id: request.paper_context_snapshot_id.clone(),
        expected_context_hash: request.expected_context_hash.clone(),
    };
    let preview = reader
        .preview(tx, &snapshot_request)
        .map_err(|flow| flow.map_error(preview_error))?;
    let mut context =
        agent_preview::submission_context_from_preview(preview).map_err(preview_error)?;
    if let Some(retrieval) = request.retrieval.as_ref() {
        context.push(
            reader
                .retrieval(
                    tx,
                    store.retrieval_trace_source(),
                    retrieval,
                    &request.paper_id,
                    &request.paper_context_snapshot_id,
                )
                .map_err(|flow| flow.map_error(preview_error))?,
        );
    }
    let preview = reader
        .preview(tx, &snapshot_request)
        .map_err(|flow| flow.map_error(preview_error))?;
    let images = agent_preview::submission_images_from_preview(preview, tx, store, None)
        .map_err(preview_error)?;
    if context.is_empty() && images.is_empty() {
        return Err(Error::InvalidInput.into());
    }
    research_agent_core::context::assemble_context_text(
        research_agent_core::context::ContextIdentity::new(
            "reading",
            request.paper_id.clone(),
            request.expected_context_hash.clone(),
        )
        .map_err(|_| Error::InvalidInput)?,
        request.request_id.to_string(),
        &context,
    )
    .map_err(|_| Error::InvalidInput)?;
    let (skills, skill_instructions) = crate::agent::skills::injection::freeze_application(
        store.agent_configuration_directory(),
        &request.text,
        &request.user_input,
    )
    .map_err(|_| Error::Unavailable)?;
    crate::agent::skills::injection::validate_mentions(store, tx, &request.user_input, &context)
        .map_err(submission_error)?;
    let (instructions, instruction_configs) = crate::agent::config::profile_snapshot::freeze(
        store.agent_configuration_directory(),
        crate::agent::config::profiles::ProfileKind::Reading,
        format!("paper:{}", request.paper_id),
        [
            request.session_config.map(Into::into),
            request.turn_config.map(Into::into),
        ],
    )
    .map_err(|_| Error::Unavailable)?;
    let record = insert_in_with_images(
        tx,
        request.conversation_id,
        request.request_id,
        SubmissionInput {
            remote_compaction_v2: crate::agent::config::freeze_remote_compaction(
                store.agent_configuration_directory(),
                &identity,
            )
            .map_err(|_| Error::Unavailable)?,
            user_input: request.user_input,
            skill_instructions,
            text: request.text,
            model: identity,
            reasoning_effort,
            service_tier,
            context,
            instructions,
            instruction_configs,
            skills,
        },
        &images,
    )
    .map_err(submission_error)?;
    crate::agent::skills::steering::freeze_grants(tx, store, &record)
        .map_err(|_| Error::Unavailable)?;
    let scope = tx
        .query_row(
            "select paper_id,paper_context_snapshot_id,context_hash from agent_paper_submission_scopes where conversation_id=?1 and request_id=?2",
            params![request.conversation_id.to_string(), request.request_id.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()
        .map_err(|_| Error::Unavailable)?;
    if let Some((paper, snapshot, hash)) = scope {
        if paper != request.paper_id
            || snapshot != request.paper_context_snapshot_id
            || hash != request.expected_context_hash
        {
            return Err(Error::Conflict.into());
        }
    } else {
        tx.execute(
            "insert into agent_paper_submission_scopes(conversation_id,request_id,turn_id,paper_id,paper_context_snapshot_id,context_hash) values(?1,?2,?3,?4,?5,?6)",
            params![
                request.conversation_id.to_string(),
                request.request_id.to_string(),
                record.turn_id.to_string(),
                request.paper_id,
                request.paper_context_snapshot_id,
                request.expected_context_hash,
            ],
        )
        .map_err(|_| Error::Unavailable)?;
    }
    Ok(record)
}

impl From<Error> for Flow<Error, ReadingNeed> {
    fn from(error: Error) -> Self {
        Self::original(error)
    }
}
