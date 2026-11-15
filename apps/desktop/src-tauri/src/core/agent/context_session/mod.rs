//! Conversation-owned context orchestration. The shared runner sees only a frozen SubmissionInput.
#[path = "catalog.rs"]
mod catalog;
#[path = "configuration.rs"]
pub(crate) mod configuration;
#[path = "docling_source.rs"]
mod docling_source;
#[path = "group_scope.rs"]
pub(crate) mod group_scope;
#[path = "permissions.rs"]
pub(crate) mod permissions;
#[path = "preparation.rs"]
mod preparation;
pub(crate) use preparation::{materialize_in, materialize_with_context};
#[path = "prepare_status.rs"]
pub(crate) mod prepare_status;
#[path = "queue.rs"]
pub(crate) mod queue;
#[path = "recommendation.rs"]
pub(crate) mod recommendation;
#[path = "references.rs"]
pub(crate) mod references;
#[path = "resource_grants.rs"]
pub(crate) mod resource_grants;
#[path = "resource_tools.rs"]
pub(crate) mod resource_tools;
#[path = "sources.rs"]
pub(crate) mod sources;
#[path = "start.rs"]
pub(crate) mod start;
#[path = "steering.rs"]
pub(crate) mod steering;
#[path = "structured.rs"]
pub(crate) mod structured;
#[path = "workspace.rs"]
pub(crate) mod workspace;
use super::{
    model::{AgentHost, AgentModelResolver, ModelSelection},
    repository::submissions,
};
use crate::storage::AppStore;
use research_agent_core::submission::{SubmissionInput, SubmissionRecord};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Serialize, thiserror::Error, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    #[error("invalid context request")]
    InvalidInput,
    #[error("independent conversation unavailable")]
    ConversationUnavailable,
    #[error("selected source unavailable or changed")]
    SourceUnavailable,
    #[error(
        "MODEL_NOTE_TEXT_VIEW_UNSTABLE: note text encoding cannot round-trip; page={page_id}; request={request_id}; preserve editor content; this is not a resource-change error"
    )]
    NoteTextViewUnstable { page_id: String, request_id: Uuid },
    #[error("open the PDF and finish text indexing before adding it")]
    PaperNotIndexed,
    #[error("the selected PDF version needs a successful Docling parse")]
    PaperNotParsed,
    #[error(
        "automatic Docling preparation failed for the selected PDF; check the local Docling service and retry"
    )]
    PaperParseFailed,
    #[error("selected context exceeds budget")]
    Budget,
    #[error("model configuration unavailable")]
    ModelUnavailable,
    #[error("confirmed snapshot does not match")]
    ConfirmationMismatch,
    #[error("context submission unavailable")]
    Unavailable,
    #[error("queue admission outcome unknown")]
    QueueAdmissionUnknown {
        conversation_id: Uuid,
        request_id: Uuid,
        phase: &'static str,
        safe_cause: &'static str,
        sqlite_code: Option<String>,
        sqlite_extended_code: Option<i32>,
    },
    #[error("the original task owner is not available for observation")]
    TaskObservationUnavailable,
    #[serde(untagged)]
    #[error("{0}")]
    NativeCatalogUnavailable(Box<super::context_catalog::ContextCatalogFailure>),
}
impl From<super::context_catalog::ContextCatalogReadError> for Error {
    fn from(error: super::context_catalog::ContextCatalogReadError) -> Self {
        use super::context_catalog::ContextCatalogReadError as E;
        match error {
            E::Unavailable => Self::Unavailable,
            E::SourceUnavailable => Self::SourceUnavailable,
            E::Native(error) => Self::NativeCatalogUnavailable(error),
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareRequest {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub text: String,
    #[serde(default)]
    pub user_input: Vec<research_agent_core::user_input::UserInput>,
    pub resources: Vec<sources::Resource>,
    pub retrieval: Option<super::retrieval_context::RetrievalSelection>,
    pub model: Option<ModelSelection>,
    pub queue: Option<super::queue_contract::Confirmation>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub conversation_id: Uuid,
    pub request_id: Uuid,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub request_id: Uuid,
    pub turn_id: Uuid,
    pub context_snapshot_id: Uuid,
    pub input: SubmissionInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<super::queue_contract::Confirmation>,
}
impl From<SubmissionRecord> for Preview {
    fn from(r: SubmissionRecord) -> Self {
        Self {
            schema_version: 1,
            conversation_id: r.conversation_id,
            request_id: r.request_id,
            turn_id: r.turn_id,
            context_snapshot_id: r.context_snapshot_id,
            input: r.input,
            queue: None,
        }
    }
}
pub(crate) fn check_conversation(db: &Connection, id: Uuid) -> Result<(), Error> {
    let allowed: bool = db.query_row("select exists(select 1 from agent_conversations c where c.id=?1 and c.archived=0 and not exists(select 1 from agent_conversation_bindings b where b.conversation_id=c.id and b.resource_kind='manuscript'))",[id.to_string()],|r|r.get(0)).map_err(|_|Error::Unavailable)?;
    if id.is_nil() || !allowed {
        Err(Error::ConversationUnavailable)
    } else {
        Ok(())
    }
}
#[tauri::command]
pub(crate) async fn agent_search_context_resources(
    store: State<'_, AppStore>,
    query: String,
) -> Result<Vec<sources::Selection>, Error> {
    search_resources(&store, query).await
}

pub(crate) async fn search_resources(
    store: &AppStore,
    query: String,
) -> Result<Vec<sources::Selection>, Error> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store.connection().map_err(|_| Error::Unavailable)?;
        sources::search(&db, &query)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

#[tauri::command]
pub(crate) async fn agent_search_context_catalog(
    store: State<'_, AppStore>,
    query: String,
) -> Result<Vec<sources::Selection>, Error> {
    search_catalog(&store, query).await
}

pub(crate) async fn search_catalog(
    store: &AppStore,
    query: String,
) -> Result<Vec<sources::Selection>, Error> {
    if (store.native_note_trace().is_some() || store.native_writing_context().is_some())
        && query.len() > 512
    {
        return Err(Error::InvalidInput);
    }
    let papers = if let Some(binding) = store.native_note_trace() {
        Some(
            binding
                .service
                .context_catalog_resources(&binding.package_key)
                .await?,
        )
    } else {
        None
    };
    let manuscripts = if let Some(binding) = store.native_writing_context() {
        Some(binding.context_catalog_resources().await?)
    } else {
        None
    };
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store.connection().map_err(|_| Error::Unavailable)?;
        let source = store.context_catalog_source();
        if let Some(manuscripts) = manuscripts.as_ref() {
            let papers = papers
                .as_ref()
                .map(|source| source as &dyn super::context_catalog::PaperCatalogReader)
                .or_else(|| {
                    source
                        .search_papers
                        .map(|_| &source as &dyn super::context_catalog::PaperCatalogReader)
                });
            return catalog::search_with_readers(&db, &query, papers, Some(manuscripts));
        }
        match papers.as_ref() {
            Some(papers) => catalog::search_with_papers(
                source,
                &db,
                &query,
                store.writing_resource_source(),
                Some(papers),
            ),
            None => catalog::search(source, &db, &query, store.writing_resource_source()),
        }
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub conversation_id: String,
    pub title: String,
}

#[tauri::command]
pub(crate) async fn agent_recommend_context(
    store: State<'_, AppStore>,
    candidate: sources::Selection,
) -> Result<sources::Selection, Error> {
    recommendation::recommend(store.inner(), candidate).await
}
#[tauri::command]
pub(crate) async fn agent_list_context_conversations(
    store: State<'_, AppStore>,
) -> Result<Vec<Conversation>, Error> {
    list_conversations(&store).await
}

pub(crate) async fn list_conversations(store: &AppStore) -> Result<Vec<Conversation>, Error> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db=store.connection().map_err(|_|Error::Unavailable)?;
        let mut stmt=db.prepare("select id,title from agent_conversations c where archived=0 and not exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=c.id) and not exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=c.id) and not exists(select 1 from agent_conversation_bindings b where b.conversation_id=c.id) order by updated_at desc,id limit 500").map_err(|_|Error::Unavailable)?;
        stmt.query_map([],|r|Ok(Conversation {conversation_id:r.get(0)?,title:r.get(1)?})).map_err(|_|Error::Unavailable)?.collect::<Result<Vec<_>,_>>().map_err(|_|Error::Unavailable)
    }).await.map_err(|_|Error::Unavailable)?
}

#[tauri::command]
pub(crate) async fn agent_prepare_context_submission(
    store: State<'_, AppStore>,
    models: State<'_, AgentModelResolver>,
    request: PrepareRequest,
) -> Result<Preview, Error> {
    prepare_context(&store, &models, request).await
}

pub(crate) async fn prepare_context(
    store: &AppStore,
    models: &AgentModelResolver,
    request: PrepareRequest,
) -> Result<Preview, Error> {
    if request.request_id.is_nil()
        || request.text.trim().is_empty()
        || request.text.len() > 1_048_576
        || request.resources.len() > 32
    {
        return Err(Error::InvalidInput);
    }
    let model = models
        .prepare(AgentHost::General, request.model.clone().map(Into::into))
        .await
        .map_err(|_| Error::ModelUnavailable)?;
    let identity = model.identity().clone();
    let reasoning_effort = model.reasoning_effort().map(str::to_owned);
    let service_tier = model.service_tier().map(str::to_owned);
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection().map_err(|_| Error::Unavailable)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::Unavailable)?;
        let preview = materialize_in(
            &tx,
            &store,
            request,
            (identity, reasoning_effort, service_tier),
        )?;
        tx.commit().map_err(|_| Error::Unavailable)?;
        Ok(preview)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

pub(super) async fn restore(store: &AppStore, receipt: Receipt) -> Result<SubmissionRecord, Error> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store.connection().map_err(|_| Error::Unavailable)?;
        check_conversation(&db, receipt.conversation_id)?;
        submissions::read(&db, receipt.conversation_id, receipt.request_id)
            .map_err(|_| Error::Unavailable)?
            .ok_or(Error::Unavailable)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}
#[tauri::command]
pub(crate) async fn agent_restore_context_submission(
    store: State<'_, AppStore>,
    request: Receipt,
) -> Result<Preview, Error> {
    restore_context(&store, request).await
}

pub(crate) async fn restore_context(store: &AppStore, request: Receipt) -> Result<Preview, Error> {
    let record = restore(store, request).await?;
    let owned = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut db = owned.connection().map_err(|_| Error::Unavailable)?;
        queue::restore_preview(&mut db, record)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod catalog_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalog_domain_tests.rs"]
mod catalog_domain_tests;

#[cfg(test)]
#[path = "catalog_runtime_tests.rs"]
mod catalog_runtime_tests;

#[cfg(test)]
#[path = "portrait_context_tests.rs"]
mod portrait_context_tests;

#[cfg(test)]
#[path = "catalog_writing_runtime_tests.rs"]
mod catalog_writing_runtime_tests;
