//! SQLite metadata adapter using the application's existing database connection.
use crate::data_interface::{IntoDatabase, SharedDatabase};
use research_agent_core::conversation::{
    ArchiveConversation, ArchiveProof, ArchiveStatus, BindingKind, CatalogCursor, CatalogError,
    CatalogFuture, CatalogMutationFuture, CatalogPage, CatalogQuery, ConversationBinding,
    ConversationCatalog, ConversationId, ConversationSummary, CreateBoundConversation,
    CreateBoundProof, CreateBoundStatus, RenameConversation, RenameProof, RenameStatus,
};
#[cfg(test)]
use rusqlite::TransactionBehavior;
use rusqlite::{Connection, OptionalExtension, params};
#[cfg(test)]
use std::sync::{Arc, Mutex};
use uuid::Uuid;
#[path = "catalog/activity.rs"]
mod activity;
pub use activity::ConversationActivity;
#[path = "catalog/archive.rs"]
mod archive;
pub(crate) use archive::{ValidatedArchive, archive_in, check_archive_in};
#[path = "catalog/creation.rs"]
mod creation;
pub(crate) use creation::{check_create_bound_in, check_create_bound_target_in, create_bound_in};
#[path = "catalog/deletion.rs"]
mod deletion;
pub(crate) use deletion::{
    DeletionSqlError, PreparedDeletion, delete_in, deletion_replay_sql, deletion_request_hash,
};
#[path = "catalog/export.rs"]
mod export;
#[path = "catalog/fork.rs"]
mod fork;
pub(crate) use fork::{PreparedFork, fork_in, validate_fork_request};
#[path = "catalog/fork_origin.rs"]
mod fork_origin;
#[path = "catalog/mutation.rs"]
mod mutation;
#[path = "catalog/rename.rs"]
mod rename;
#[path = "catalog/rollback_authority.rs"]
mod rollback_authority;
pub(crate) use rename::{check_rename_in, rename_in};
pub(crate) use rollback_authority::recheck_in as recheck_rollback_authority_in;
pub(crate) use rollback_authority::{AuthorizationError, authorize_in as authorize_rollback_in};
#[path = "catalog/search.rs"]
mod search;
#[cfg(test)]
#[path = "catalog/test_bindings.rs"]
mod test_bindings;
#[path = "catalog/worker_retention.rs"]
mod worker_retention;
pub use export::SnapshotExportError;
pub(crate) use worker_retention::WorkerRetention;

#[derive(Clone)]
pub struct SqliteConversationCatalog {
    connection: SharedDatabase,
    workspace_directory: Option<std::path::PathBuf>,
    worker_retention: worker_retention::WorkerRetention,
}
impl SqliteConversationCatalog {
    pub(crate) fn new(connection: impl IntoDatabase) -> Self {
        Self {
            connection: connection.into_database(),
            workspace_directory: None,
            worker_retention: worker_retention::WorkerRetention::default(),
        }
    }
    pub(crate) fn from_app_store(store: &crate::storage::AppStore) -> Self {
        Self {
            workspace_directory: Some(store.agent_workspace_directory().to_path_buf()),
            ..Self::new(store.shared_agent_connection())
        }
    }
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T, CatalogError> + Send + 'static,
    ) -> Result<T, CatalogError> {
        let connection = self.connection.clone();
        self.worker_retention
            .spawn(move || {
                let mut connection = connection.lock().map_err(|_| CatalogError::Unavailable)?;
                operation(&mut connection)
            })
            .await
            .map_err(|_| CatalogError::Unavailable)?
    }
}
impl ConversationCatalog for SqliteConversationCatalog {
    fn create_bound(
        &self,
        command: CreateBoundConversation,
    ) -> CatalogMutationFuture<'_, CreateBoundProof> {
        Box::pin(self.create_owned(command))
    }
    fn create_bound_status(
        &self,
        command: CreateBoundConversation,
    ) -> CatalogMutationFuture<'_, CreateBoundStatus> {
        Box::pin(self.inspect_creation(command))
    }
    fn read(
        &self,
        id: ConversationId,
        binding: Option<ConversationBinding>,
    ) -> CatalogFuture<'_, ConversationSummary> {
        Box::pin(async move {
            if id.as_uuid().is_nil() {
                return Err(CatalogError::InvalidInput);
            }
            if let Some(binding) = &binding {
                validate_binding(binding)?;
            }
            self.run(move |connection| {
                let row = connection.query_row(
                    "select id, title, archived, durable_count, created_at, updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=agent_conversations.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=agent_conversations.id)) from agent_conversations
                     where id = ?1 and (?2 is null or exists(select 1 from agent_conversation_bindings b
                     where b.conversation_id = agent_conversations.id and b.resource_kind = ?2 and b.resource_id = ?3))",
                    params![id.as_uuid().to_string(), binding.as_ref().map(|b| binding_kind(b.kind)), binding.as_ref().map(|b| b.resource_id.as_str())],
                    read_row,
                ).optional().map_err(db_error)?.ok_or(CatalogError::NotFound)?;
                decode(row)
            }).await
        })
    }
    fn set_archived(
        &self,
        command: ArchiveConversation,
    ) -> CatalogMutationFuture<'_, ArchiveProof> {
        Box::pin(self.archive_owned(command))
    }
    fn archive_status(
        &self,
        command: ArchiveConversation,
    ) -> CatalogMutationFuture<'_, ArchiveStatus> {
        Box::pin(self.inspect_archive(command))
    }
    fn list(&self, query: CatalogQuery) -> CatalogFuture<'_, CatalogPage> {
        Box::pin(async move {
            if query.limit == 0 || query.limit > 500 {
                return Err(CatalogError::InvalidInput);
            }
            if let Some(binding) = &query.binding {
                validate_binding(binding)?;
            }
            self.run(move |connection| {
                let mut statement = connection.prepare(
                    "select id, title, archived, durable_count, created_at, updated_at, (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=agent_conversations.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=agent_conversations.id)) from agent_conversations
                     where archived = ?1 and (exists(select 1 from agent_paper_analysis_bindings internal where internal.conversation_id=agent_conversations.id) or exists(select 1 from agent_legacy_ai_run_bindings internal where internal.conversation_id=agent_conversations.id)) = ?7 and (?2 is null or created_at < ?2 or (created_at = ?2 and id < ?3))
                     and (?5 is null or exists(select 1 from agent_conversation_bindings b where b.conversation_id = agent_conversations.id and b.resource_kind = ?5 and b.resource_id = ?6))
                     order by created_at desc, id desc limit ?4"
                ).map_err(db_error)?;
                let rows = statement.query_map(params![query.archived, query.before.as_ref().map(|c| c.created_at.as_str()), query.before.as_ref().map(|c| c.id.as_uuid().to_string()), i64::from(query.limit) + 1, query.binding.as_ref().map(|b| binding_kind(b.kind)), query.binding.as_ref().map(|b| b.resource_id.as_str()), query.internal], read_row).map_err(db_error)?;
                let mut conversations = rows.map(|row| decode(row.map_err(db_error)?)).collect::<Result<Vec<_>, _>>()?;
                let has_more = conversations.len() > usize::from(query.limit);
                conversations.truncate(usize::from(query.limit));
                let next = if has_more { conversations.last().map(|last| CatalogCursor { created_at: last.created_at.clone(), id: last.id }) } else { None };
                Ok(CatalogPage { conversations, next })
            }).await
        })
    }
    fn rename(&self, command: RenameConversation) -> CatalogMutationFuture<'_, RenameProof> {
        Box::pin(self.rename_owned(command))
    }
    fn rename_status(
        &self,
        command: RenameConversation,
    ) -> CatalogMutationFuture<'_, RenameStatus> {
        Box::pin(self.inspect_rename(command))
    }
}
type Row = (String, String, bool, i64, String, String, bool);
fn binding_kind(kind: BindingKind) -> &'static str {
    match kind {
        BindingKind::Manuscript => "manuscript",
        BindingKind::Paper => "paper",
        BindingKind::Note => "note",
        BindingKind::ReadingNote => "readingNote",
    }
}
/// All association entry points use the same closed resource-kind lookup.
pub(crate) fn binding_target_query(kind: BindingKind) -> &'static str {
    match kind {
        BindingKind::Manuscript => "select exists(select 1 from manuscripts where id=?1)",
        BindingKind::Paper => "select exists(select 1 from bibliographic_items where id=?1)",
        BindingKind::Note => {
            "select exists(select 1 from note_pages p where p.id=?1 and not exists(select 1 from reading_note_sources s where s.page_id=p.id))"
        }
        BindingKind::ReadingNote => {
            "select exists(select 1 from reading_note_sources where block_id=?1)"
        }
    }
}
fn validate_binding(binding: &ConversationBinding) -> Result<(), CatalogError> {
    if binding.resource_id.trim().is_empty()
        || binding.resource_id.chars().count() > 256
        || binding.resource_id.contains('\0')
    {
        return Err(CatalogError::InvalidInput);
    }
    Ok(())
}
fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}
fn decode(
    (id, title, archived, count, created_at, updated_at, internal): Row,
) -> Result<ConversationSummary, CatalogError> {
    Ok(ConversationSummary {
        internal,
        id: ConversationId::from_uuid(Uuid::parse_str(&id).map_err(|_| CatalogError::Corrupt)?),
        title,
        archived,
        durable_count: u64::try_from(count).map_err(|_| CatalogError::Corrupt)?,
        created_at,
        updated_at,
    })
}
fn db_error(_: rusqlite::Error) -> CatalogError {
    CatalogError::Unavailable
}

#[cfg(test)]
#[path = "catalog/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalog/creation_tests.rs"]
mod creation_tests;
#[cfg(test)]
#[path = "catalog/search_performance_tests.rs"]
mod search_performance_tests;
#[cfg(test)]
#[path = "catalog/search_tests.rs"]
mod search_tests;

#[cfg(test)]
#[path = "catalog/reading_note_tests.rs"]
mod reading_note_tests;
