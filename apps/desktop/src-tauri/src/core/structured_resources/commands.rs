//! Thin desktop IPC over the same resource service used by Agent tools.
use super::*;
use crate::runtime_plugins::{
    literature_hbs::ResourceOperationError, literature_trace::service::LiteratureTraceService,
    native::NativeHostError,
};
use crate::storage::AppStore;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct CommandError(CommandErrorKind);
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum CommandErrorKind {
    Core { code: &'static str, message: String },
    Native(Box<NativeHostError>),
    Resource(ResourceOperationError),
}
impl CommandError {
    fn core(code: &'static str, message: String) -> Self {
        Self(CommandErrorKind::Core { code, message })
    }
}
impl From<Error> for CommandError {
    fn from(error: Error) -> Self {
        Self::core(error.command_code(), error.to_string())
    }
}
impl From<Box<NativeHostError>> for CommandError {
    fn from(error: Box<NativeHostError>) -> Self {
        Self(CommandErrorKind::Native(error))
    }
}

#[tauri::command]
pub async fn structured_resource_import_startup(
    store: State<'_, AppStore>,
    service: State<'_, Arc<LiteratureTraceService>>,
    draft: StartupImportDraft,
) -> Result<StartupImportResult, CommandError> {
    if let Some(source) = store.paper_reading_source() {
        return import_startup(
            source,
            &mut *store.connection()?,
            draft,
            store.writing_resource_source(),
        )
        .map_err(Into::into);
    }
    service
        .structured_startup(
            store.shared_agent_connection(),
            explicit_key(&store),
            draft,
            store.writing_resource_source(),
        )
        .await
        .map_err(Into::into)
}

impl From<crate::ai::error::AiError> for CommandError {
    fn from(error: crate::ai::error::AiError) -> Self {
        if let crate::ai::error::AiError::NativeWorkflow { failure, .. } = error {
            return failure.into();
        }
        let code = error.code();
        let error = crate::ai::error::AiCommandError::from(error);
        Self::core(code, error.message().to_owned())
    }
}
impl From<crate::storage::StorageError> for CommandError {
    fn from(error: crate::storage::StorageError) -> Self {
        if let crate::storage::StorageError::QueryWorker(cause) = error {
            return Self::core(
                "db_query_worker_failed",
                format!("结构化资源查询工作线程失败：{cause}"),
            );
        }
        Self::core("storage_error", "结构化资源存储不可用".into())
    }
}

#[tauri::command]
pub async fn structured_list_resources(
    store: State<'_, AppStore>,
    paper_id: Option<String>,
) -> Result<Vec<Resource>, CommandError> {
    store
        .query(move |db| {
            if let Some(paper) = paper_id.as_deref() {
                ensure_paper_resources(db, paper)?;
            }
            let mut resources = list(db, paper_id.as_deref())?;
            for resource in &mut resources {
                attach_latest_draft(db, resource)?;
            }
            Ok(resources)
        })
        .await
}
#[tauri::command]
pub fn structured_get_resource(
    store: State<'_, AppStore>,
    resource_id: String,
) -> Result<Resource, CommandError> {
    let db = store.connection()?;
    let mut resource = get(&db, &resource_id)?;
    attach_latest_draft(&db, &mut resource)?;
    Ok(resource)
}
pub(crate) fn attach_latest_draft(db: &Connection, resource: &mut Resource) -> Result<(), Error> {
    let resource_id = &resource.id;
    let owner=db.query_row("select creator_conversation_id from structured_resource_drafts where resource_id=?1 order by updated_at desc,id desc limit 1",[resource_id],|r|r.get::<_,Option<String>>(0)).optional()?;
    if let Some(owner) = owner {
        resource.draft = repository::get_draft(db, resource_id, owner.as_deref())?;
    }
    Ok(())
}
#[tauri::command]
pub async fn structured_get_resource_version(
    store: State<'_, AppStore>,
    resource_id: String,
    revision: i64,
) -> Result<Resource, CommandError> {
    store
        .query(move |db| get_version(db, &resource_id, revision).map_err(Into::into))
        .await
}
#[tauri::command]
pub fn structured_create_template(
    store: State<'_, AppStore>,
    title: String,
    request_id: String,
    conversation_id: Option<String>,
) -> Result<Resource, CommandError> {
    create_template(
        &*store.connection()?,
        &title,
        &request_id,
        conversation_id.as_deref(),
    )
    .map_err(Into::into)
}
#[tauri::command]
pub async fn structured_write_resource(
    store: State<'_, AppStore>,
    service: State<'_, Arc<LiteratureTraceService>>,
    draft: WriteDraft,
) -> Result<WriteResult, CommandError> {
    let store = store.inner().clone();
    if store.generation_policy_source().is_some() || store.paper_reading_source().is_some() {
        return tokio::task::spawn_blocking(move || {
            let result = write(
                store.require_generation_policy_source()?,
                store.require_paper_reading_source()?,
                &mut *store.connection()?,
                draft,
            )?;
            store.notify_structured_resource_changed(
                &result.resource.id,
                result.resource.paper_id.as_deref(),
            );
            Ok(result)
        })
        .await
        .map_err(|_| unsettled_review_worker())?;
    }
    service
        .write_resource(store, draft)
        .await
        .map_err(|error| CommandError(CommandErrorKind::Resource(error)))
}
#[tauri::command]
pub fn structured_patch_resource(
    store: State<'_, AppStore>,
    draft: PatchDraft,
) -> Result<WriteResult, CommandError> {
    let result = patch(
        store.require_generation_policy_source()?,
        store.require_paper_reading_source()?,
        &mut *store.connection()?,
        draft,
    )?;
    store.notify_structured_resource_changed(
        &result.resource.id,
        result.resource.paper_id.as_deref(),
    );
    Ok(result)
}
#[tauri::command]
pub async fn structured_review_resource(
    store: State<'_, AppStore>,
    service: State<'_, Arc<LiteratureTraceService>>,
    draft: ReviewDraft,
) -> Result<Resource, CommandError> {
    let store = store.inner().clone();
    if store.generation_policy_source().is_some() || store.paper_reading_source().is_some() {
        return tokio::task::spawn_blocking(move || {
            let result = review(
                store.require_generation_policy_source()?,
                store.require_paper_reading_source()?,
                &mut *store.connection()?,
                draft,
            )?;
            store.notify_structured_resource_changed(&result.id, result.paper_id.as_deref());
            Ok(result)
        })
        .await
        .map_err(|_| unsettled_review_worker())?;
    }
    service
        .review_resource(store, draft)
        .await
        .map_err(|error| CommandError(CommandErrorKind::Resource(error)))
}
#[tauri::command]
pub async fn structured_restore_resource_history(
    store: State<'_, AppStore>,
    service: State<'_, Arc<LiteratureTraceService>>,
    draft: HistoryDraft,
) -> Result<Resource, CommandError> {
    let store = store.inner().clone();
    if store.generation_policy_source().is_some() || store.paper_reading_source().is_some() {
        return tokio::task::spawn_blocking(move || {
            let mut db = store.connection()?;
            let mut result = history::restore(
                store.require_generation_policy_source()?,
                store.require_paper_reading_source()?,
                &mut db,
                draft,
            )?;
            attach_latest_draft(&db, &mut result)?;
            store.notify_structured_resource_changed(&result.id, result.paper_id.as_deref());
            Ok(result)
        })
        .await
        .map_err(|_| unsettled_review_worker())?;
    }
    service
        .restore_resource_history(store, draft)
        .await
        .map_err(|error| CommandError(CommandErrorKind::Resource(error)))
}
fn unsettled_review_worker() -> CommandError {
    CommandError::core(
        "storage_error",
        "结构化资源工作线程未确认结算；原请求与内容已保留。".into(),
    )
}
#[tauri::command]
pub async fn structured_bind_template(
    store: State<'_, AppStore>,
    service: State<'_, Arc<LiteratureTraceService>>,
    draft: AdoptionDraft,
) -> Result<Resource, CommandError> {
    let store = store.inner().clone();
    if store.generation_policy_source().is_some() || store.paper_reading_source().is_some() {
        return tokio::task::spawn_blocking(move || {
            let result = adopt_template(
                store.require_generation_policy_source()?,
                store.require_paper_reading_source()?,
                &mut *store.connection()?,
                draft,
            )?;
            store.notify_structured_resource_changed(&result.id, result.paper_id.as_deref());
            Ok(result)
        })
        .await
        .map_err(|_| unsettled_review_worker())?;
    }
    service
        .bind_resource_template(store, draft)
        .await
        .map_err(|error| CommandError(CommandErrorKind::Resource(error)))
}
#[tauri::command]
pub async fn structured_import_legacy(
    store: State<'_, AppStore>,
    service: State<'_, Arc<LiteratureTraceService>>,
    draft: LegacyImportDraft,
) -> Result<Vec<Resource>, CommandError> {
    if let Some(source) = store.paper_reading_source() {
        return import_legacy(source, &mut *store.connection()?, draft).map_err(Into::into);
    }
    service
        .structured_legacy(store.shared_agent_connection(), explicit_key(&store), draft)
        .await
        .map_err(Into::into)
}

pub(crate) fn sources(
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &Connection,
    id: &str,
) -> Result<Value, Error> {
    sources_with_evidence(&reading_source, db, id)
}
pub(crate) fn sources_with_evidence(
    reading_source: &dyn crate::ai::structured_resource_validation::ExplicitReadingEvidence,
    db: &Connection,
    id: &str,
) -> Result<Value, Error> {
    super::provenance::sources(reading_source, db, &get(db, id)?)
}
fn explicit_key(store: &AppStore) -> Option<String> {
    store
        .native_note_trace()
        .map(|binding| binding.package_key.clone())
}
#[tauri::command]
pub async fn structured_get_resource_sources(
    store: State<'_, AppStore>,
    service: State<'_, Arc<LiteratureTraceService>>,
    resource_id: String,
) -> Result<Value, CommandError> {
    if let Some(source) = store.paper_reading_source() {
        return store
            .query(move |db| sources(source, db, &resource_id).map_err(Into::into))
            .await;
    }
    service
        .structured_sources(
            store.shared_agent_connection(),
            explicit_key(&store),
            resource_id,
        )
        .await
        .map_err(Into::into)
}
