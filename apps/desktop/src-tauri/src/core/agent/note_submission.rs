//! Saved-note host admission to the shared immutable submission store.
use super::{
    model::{AgentHost, AgentModelResolver, ModelSelection},
    repository::submissions::{insert_in, read},
    retrieval_context::{RetrievalScope, RetrievalSelection, consume_in},
};
use crate::{
    agent::config::ConfigOverride, notes::agent_context::NoteContextRequest, storage::AppStore,
};
use research_agent_core::submission::{ContextSourceKind, SubmissionInput, SubmissionRecord};
use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[path = "note_submission/native_diagnostic.rs"]
mod native_diagnostic;
#[path = "note_submission/native_trace.rs"]
mod native_trace;
#[path = "note_submission/profile.rs"]
mod profile;
#[path = "note_submission/recovery.rs"]
pub(crate) mod recovery;
#[path = "note_submission/source.rs"]
pub(super) mod source;
use source::{Resource, Selection, SourceReceipt};

pub(super) struct SelectedRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub text: String,
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
    pub context: Selection,
    pub retrieval: Option<RetrievalSelection>,
    pub model: Option<ModelSelection>,
    pub session_config: Option<research_agent_core::config::InstructionConfigPatch>,
    pub turn_config: Option<research_agent_core::config::InstructionConfigPatch>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
    pub context: NoteContextRequest,
    #[serde(default)]
    pub retrieval: Option<RetrievalSelection>,
    pub model: Option<ModelSelection>,
    #[serde(default)]
    pub session_config: Option<ConfigOverride>,
    #[serde(default)]
    pub turn_config: Option<ConfigOverride>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub page_id: Uuid,
}

#[derive(Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    #[error("invalid note submission")]
    InvalidInput,
    #[error("note conversation binding unavailable")]
    Unbound,
    #[error("note submission changed; refresh the selected source")]
    Conflict,
    #[error("note submission source exceeds its budget")]
    Budget,
    #[error("note model configuration unavailable")]
    ModelUnavailable,
    #[error("note instruction profile configuration unavailable")]
    ProfileUnavailable,
    #[error("note submission unavailable")]
    Unavailable,
    #[serde(untagged)]
    #[error(transparent)]
    Native(Box<native_diagnostic::NativeDiagnostic>),
}

/// Prepare local disclosure review only. Model credentials never leave the
/// resolver; no inference, tool execution, note mutation or permission grant.
pub(crate) async fn prepare(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
) -> Result<SubmissionRecord, Error> {
    prepare_selected(
        store,
        models,
        SelectedRequest {
            user_input: request.user_input,
            conversation_id: request.conversation_id,
            request_id: request.request_id,
            text: request.text,
            context: Selection::Page(request.context),
            retrieval: request.retrieval,
            model: request.model,
            session_config: request.session_config.map(Into::into),
            turn_config: request.turn_config.map(Into::into),
        },
    )
    .await
}

pub(super) async fn prepare_selected(
    store: &AppStore,
    models: &AgentModelResolver,
    request: SelectedRequest,
) -> Result<SubmissionRecord, Error> {
    prepare_selected_with_route(
        store,
        models,
        request,
        RetrievalPreparation::NativePage,
        |tx, receipt, input| {
            insert_in(tx, receipt.conversation_id, receipt.request_id, input)
                .map_err(submission_error)
        },
    )
    .await
}

enum RetrievalPreparation {
    Existing,
    NativePage,
}

/// Source/config freezing is shared by normal and queued submissions. Persistence
/// runs inside the same authorized transaction and cannot be supplied by the UI.
pub(super) async fn prepare_selected_with<T, E, F>(
    store: &AppStore,
    models: &AgentModelResolver,
    request: SelectedRequest,
    persist: F,
) -> Result<T, E>
where
    T: Send + 'static,
    E: From<Error> + Send + 'static,
    F: FnOnce(&rusqlite::Transaction<'_>, SourceReceipt, SubmissionInput) -> Result<T, E>
        + Send
        + 'static,
{
    // Queued admission has a different persist contract and is not part of the
    // finite ordinary-Note native trace migration.
    prepare_selected_with_route(
        store,
        models,
        request,
        RetrievalPreparation::Existing,
        persist,
    )
    .await
}

async fn prepare_selected_with_route<T, E, F>(
    store: &AppStore,
    models: &AgentModelResolver,
    request: SelectedRequest,
    retrieval_route: RetrievalPreparation,
    persist: F,
) -> Result<T, E>
where
    T: Send + 'static,
    E: From<Error> + Send + 'static,
    F: FnOnce(&rusqlite::Transaction<'_>, SourceReceipt, SubmissionInput) -> Result<T, E>
        + Send
        + 'static,
{
    let receipt = SourceReceipt {
        conversation_id: request.conversation_id,
        request_id: request.request_id,
        resource: request.context.resource(),
    };
    receipt.validate()?;
    request.context.validate().map_err(context_error)?;
    if request.text.trim().is_empty() || request.text.len() > 1_048_576 {
        return Err(Error::InvalidInput.into());
    }
    let check_store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = check_store.connection().map_err(|_| Error::Unavailable)?;
        receipt.check_binding(&db)
    })
    .await
    .map_err(|_| Error::Unavailable)??;
    let model = models
        .prepare(AgentHost::Notes, request.model.clone().map(Into::into))
        .await
        .map_err(|_| Error::ModelUnavailable)?;
    let identity = model.identity().clone();
    let reasoning_effort = model.reasoning_effort().map(str::to_owned);
    let service_tier = model.service_tier().map(str::to_owned);
    drop(model);
    let native_trace = match retrieval_route {
        RetrievalPreparation::Existing => None,
        RetrievalPreparation::NativePage => native_trace::prepare(store, &request, receipt).await?,
    };
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::Unavailable)?;
        receipt.check_binding(&tx)?;
        let mut contexts =
            source::materialize_contexts(&tx, &request.context, &request.user_input)?;
        if let Some(retrieval) = request.retrieval.as_ref() {
            let resource = request.context.resource();
            let resource_id = resource.id().to_string();
            let retrieved = if let Some(prepared) = &native_trace {
                retrieval.validate().map_err(|_| Error::InvalidInput)?;
                prepared.consume_in(&tx)?
            } else {
                consume_in(
                    store.retrieval_trace_source().ok_or(Error::Unavailable)?,
                    &tx,
                    retrieval,
                    RetrievalScope::Note {
                        resource_id: &resource_id,
                        anchor_key: resource.anchor_key(),
                    },
                )
                .map_err(|error| match error {
                    super::retrieval_context::RetrievalContextError::Invalid => Error::InvalidInput,
                    super::retrieval_context::RetrievalContextError::Conflict => Error::Conflict,
                })?
            };
            contexts.push(retrieved);
        }
        research_agent_core::context::assemble_context_text(
            research_agent_core::context::ContextIdentity::new(
                "notes",
                receipt.resource.id().to_string(),
                contexts[0].revision.clone(),
            )
            .map_err(|_| Error::InvalidInput)?,
            request.request_id.to_string(),
            &contexts,
        )
        .map_err(|_| Error::InvalidInput)?;
        let (instructions, instruction_configs) = profile::freeze(
            store.agent_configuration_directory(),
            receipt.resource,
            [request.session_config, request.turn_config],
        )?;
        let (skills, skill_instructions) = crate::agent::skills::injection::freeze_application(
            store.agent_configuration_directory(),
            &request.text,
            &request.user_input,
        )
        .map_err(|_| Error::Unavailable)?;
        crate::agent::skills::injection::validate_mentions(
            &store,
            &tx,
            &request.user_input,
            &contexts,
        )
        .map_err(|_| Error::Conflict)?;
        let result = persist(
            &tx,
            receipt,
            SubmissionInput {
                remote_compaction_v2: super::config::freeze_remote_compaction(
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
                context: contexts,
                instructions,
                instruction_configs,
                skills,
            },
        )?;
        if let Some(record) =
            read(&tx, receipt.conversation_id, receipt.request_id).map_err(submission_error)?
        {
            crate::agent::skills::steering::freeze_grants(&tx, &store, &record)
                .map_err(|_| Error::Unavailable)?;
        }
        tx.commit().map_err(|_| Error::Unavailable)?;
        Ok::<_, E>(result)
    })
    .await
    .map_err(|_| E::from(Error::Unavailable))?
}

/// Recover the original review without consulting current note content or model
/// defaults. Reading a receipt never starts/restarts its turn or refreshes auth.
pub(crate) async fn restore(store: &AppStore, receipt: Receipt) -> Result<SubmissionRecord, Error> {
    validate_receipt(receipt)?;
    restore_selected(
        store,
        SourceReceipt {
            conversation_id: receipt.conversation_id,
            request_id: receipt.request_id,
            resource: Resource::Page(receipt.page_id),
        },
    )
    .await
}

pub(super) async fn restore_selected(
    store: &AppStore,
    receipt: SourceReceipt,
) -> Result<SubmissionRecord, Error> {
    receipt.validate()?;
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db.transaction().map_err(|_| Error::Unavailable)?;
        receipt.check_binding(&tx)?;
        let record = read(&tx, receipt.conversation_id, receipt.request_id)
            .map_err(submission_error)?
            .ok_or(Error::Unavailable)?;
        validate_record(&record, receipt.resource)?;
        Ok(record)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub(super) fn validate_record(record: &SubmissionRecord, resource: Resource) -> Result<(), Error> {
    if record.input.context.iter().any(|source| {
        source.kind != ContextSourceKind::Note && source.kind != ContextSourceKind::Retrieval
    }) {
        return Err(Error::Unbound);
    }
    if record
        .input
        .context
        .iter()
        .filter(|source| source.kind == ContextSourceKind::Note)
        .count()
        != 1
        || record
            .input
            .context
            .iter()
            .filter(|source| source.kind == ContextSourceKind::Retrieval)
            .count()
            > 1
    {
        return Err(Error::Unbound);
    }
    let source = record
        .input
        .context
        .iter()
        .find(|source| source.kind == ContextSourceKind::Note)
        .ok_or(Error::Unbound)?;
    let anchor: serde_json::Value =
        serde_json::from_str(source.anchor.as_deref().ok_or(Error::Unavailable)?)
            .map_err(|_| Error::Unavailable)?;
    if source.kind != ContextSourceKind::Note
        || source.resource_id != resource.id().to_string()
        || anchor
            .get(resource.anchor_key())
            .and_then(serde_json::Value::as_str)
            != Some(resource.id().to_string().as_str())
    {
        return Err(Error::Unbound);
    }
    if matches!(resource, Resource::Reading(_))
        && source.source_id != format!("reading-note:{}", resource.id())
    {
        return Err(Error::Unbound);
    }
    for retrieval in record
        .input
        .context
        .iter()
        .filter(|source| source.kind == ContextSourceKind::Retrieval)
    {
        let anchor: serde_json::Value =
            serde_json::from_str(retrieval.anchor.as_deref().ok_or(Error::Unavailable)?)
                .map_err(|_| Error::Unavailable)?;
        let trace_id = anchor
            .get("traceId")
            .and_then(serde_json::Value::as_str)
            .ok_or(Error::Unbound)?;
        let query = anchor
            .get("query")
            .and_then(serde_json::Value::as_str)
            .ok_or(Error::Unbound)?;
        if trace_id.is_empty()
            || query.is_empty()
            || retrieval.resource_id != resource.id().to_string()
            || retrieval.revision != format!("trace:{trace_id}")
            || retrieval.source_id != format!("retrieval:{trace_id}")
            || anchor
                .get(resource.anchor_key())
                .and_then(serde_json::Value::as_str)
                != Some(resource.id().to_string().as_str())
        {
            return Err(Error::Unbound);
        }
    }
    profile::validate(&record.input, resource)
}

fn validate_receipt(receipt: Receipt) -> Result<(), Error> {
    if receipt.conversation_id.is_nil() || receipt.request_id.is_nil() || receipt.page_id.is_nil() {
        Err(Error::InvalidInput)
    } else {
        Ok(())
    }
}

pub(super) fn check_binding(
    db: &Connection,
    conversation_id: Uuid,
    page_id: Uuid,
) -> Result<(), Error> {
    let allowed: bool = db.query_row(
        "select exists(select 1 from agent_conversation_bindings b
         join agent_conversations c on c.id=b.conversation_id join note_pages n on n.id=b.resource_id
         where b.conversation_id=?1 and b.resource_kind='note' and b.resource_id=?2
         and c.archived=0 and n.status='active'
         and not exists(select 1 from reading_note_sources s where s.page_id=n.id))", params![conversation_id.to_string(),page_id.to_string()],
        |row| row.get(0),
    ).map_err(|_| Error::Unavailable)?;
    if allowed { Ok(()) } else { Err(Error::Unbound) }
}

fn context_error(error: crate::notes::agent_context::NoteContextError) -> Error {
    use crate::notes::agent_context::NoteContextError;
    match error {
        NoteContextError::InvalidInput => Error::InvalidInput,
        NoteContextError::Conflict => Error::Conflict,
        NoteContextError::Budget => Error::Budget,
        NoteContextError::Unavailable => Error::Unavailable,
    }
}
fn submission_error(error: research_agent_core::submission::SubmissionError) -> Error {
    use research_agent_core::submission::SubmissionError;
    match error {
        SubmissionError::InvalidInput => Error::InvalidInput,
        SubmissionError::Conflict => Error::Conflict,
        _ => Error::Unavailable,
    }
}

#[cfg(test)]
#[path = "note_submission/tests.rs"]
mod tests;
