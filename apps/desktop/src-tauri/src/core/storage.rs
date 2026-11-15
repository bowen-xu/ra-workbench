use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use rusqlite::Connection;
use tauri::{AppHandle, Manager};

#[cfg(test)]
use crate::data_interface::IntoDatabase;
use crate::{
    data_interface::{Database, DatabaseGuard, SharedDatabase},
    storage_backup::backup_before_pending_migrations_locked,
    storage_environment::validate_data_environment,
    storage_integrity::{
        backfill_migration_checksums, record_migration_checksum, validate_ai_candidate_schema,
        verify_recorded_migration_checksum,
    },
    storage_migrations::{MIGRATIONS, MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED},
};
use thiserror::Error;

#[path = "storage_content_session.rs"]
mod content_session;
#[path = "storage_publication_proofs.rs"]
mod publication_proofs;
#[cfg(test)]
#[path = "storage_reasoning_tests.rs"]
mod reasoning_tests;
#[path = "storage_runtime.rs"]
mod runtime;
#[cfg(test)]
#[path = "storage_runtime_test_sources.rs"]
mod runtime_test_sources;
#[path = "storage_schema_compat.rs"]
pub(crate) mod schema_compat;
#[path = "storage_startup.rs"]
mod startup;
#[cfg(test)]
#[path = "storage_writing_context_tests.rs"]
mod writing_context_tests;
#[cfg(unix)]
#[path = "storage_writing_workspace.rs"]
mod writing_workspace;
pub(crate) use runtime::{AppStorePaths, AppStorePorts, AppStoreSources};

#[derive(Debug, Error)]
pub(crate) enum StorageError {
    #[error("应用数据环境配置不安全：{0}")]
    UnsafeDataEnvironment(String),
    #[error("无法定位应用数据目录：{0}")]
    AppData(#[from] tauri::Error),
    #[error("无法创建应用数据目录：{0}")]
    CreateDirectory(#[from] std::io::Error),
    #[error("数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("数据库迁移完整性检查失败：{0}")]
    MigrationIntegrity(String),
    #[error("数据库正被另一个操作占用")]
    DatabaseBusy,
    #[error("数据库查询工作线程失败：{0}")]
    QueryWorker(String),
    #[error("文献插件启动恢复失败：{0:?}")]
    NativeStartup(Box<crate::runtime_plugins::native::NativeHostError>),
    #[error("无法读取对象存储可用空间：{0}")]
    DiskSpace(String),
    #[error("数据接口失败：{0}")]
    DataInterface(#[from] crate::data_interface::DataError),
    #[error("对象存储可用空间不足（需要保留至少 512 MiB 安全余量）")]
    InsufficientDiskSpace,
}

/// App-selected domain recovery, never a generic plugin callback API.
pub(crate) type CandidateRecoverySource = fn(&mut Connection) -> Result<(), StorageError>;

#[derive(Clone)]
pub(crate) struct AppStore {
    events: Option<crate::runtime_events::RuntimeEvents>,
    pub(crate) docling:
        Arc<std::sync::OnceLock<Arc<crate::docling::preparation::DoclingPreparation>>>,
    connection: SharedDatabase,
    content_io: Arc<RwLock<()>>,
    content_session_retained: Arc<Mutex<HashSet<String>>>,
    bibliography_io: Arc<Mutex<()>>,
    bibliography_publications: Arc<publication_proofs::PublicationProofCache>,
    writing_context_source: Option<crate::agent::writing_context::WritingContextSource>,
    writing_resource_source: Option<crate::agent::writing_resources::WritingResourceSource>,
    paper_metadata_source: Option<crate::agent::paper_metadata::PaperMetadataSource>,
    retrieval_trace_source: Option<crate::agent::retrieval_context::RetrievalTraceSource>,
    retrieval_preview_source:
        Option<Arc<crate::runtime_plugins::literature_retrieval::source::RetrievalPreviewSource>>,
    semantic_index_source:
        Option<Arc<crate::runtime_plugins::literature_index::source::SemanticIndexSource>>,
    native_note_trace: Option<crate::runtime_plugins::literature_trace::binding::NoteTraceBinding>,
    native_writing_context: Option<crate::runtime_plugins::writing_projects::context::Binding>,
    paper_portrait_source: Option<crate::agent::paper_portrait::PaperPortraitSource>,
    context_catalog_source: crate::agent::context_catalog::ContextCatalogSource,
    generation_policy_source: Option<crate::ai::generation_policy::GenerationPolicySource>,
    paper_generation_source: Option<crate::ai::paper_generation_admission::PaperGenerationSource>,
    paper_reading_source: Option<crate::ai::paper_reading_source::PaperReadingSource>,
    paper_analysis_source: Option<crate::ai::analysis_source::PaperAnalysisSource>,
    writing_compile_feedback_source:
        Option<crate::agent::compile_feedback::WritingCompileFeedbackSource>,
    #[cfg(unix)]
    writing_workspace_source: Option<crate::agent::writing_workspace::WritingWorkspaceSource>,
    #[cfg(target_os = "macos")]
    writing_workspace_compile_source:
        Option<crate::agent::workspace_compile::WritingWorkspaceCompileSource>,
    content_directory: PathBuf,
    manuscript_directory: PathBuf,
    semantic_index_directory: PathBuf,
    writing_cache_directory: PathBuf,
    agent_configuration_directory: PathBuf,
    agent_workspace_directory: PathBuf,
    text_editors: Arc<crate::agent::text_tools::editor_barrier::EditorBarrier>,
    text_patch_boot: uuid::Uuid,
    system_document_approvals: Arc<crate::system_documents::approvals::Approvals>,
    writing_delivery_tools: Arc<crate::agent::writing_delivery_tool::DeliveryTools>,
}

impl AppStore {
    pub(crate) fn with_native_writing_context(
        mut self,
        binding: crate::runtime_plugins::writing_projects::context::Binding,
    ) -> Self {
        self.native_writing_context = Some(binding);
        self
    }

    pub(crate) fn native_writing_context(
        &self,
    ) -> Option<&crate::runtime_plugins::writing_projects::context::Binding> {
        self.native_writing_context.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn without_static_writing_context_sources(mut self) -> Self {
        self.native_writing_context = None;
        self.writing_context_source = None;
        self.writing_resource_source = None;
        self.context_catalog_source.search_manuscripts = None;
        self
    }

    #[cfg(all(test, unix))]
    pub(crate) fn without_static_writing_workspace_source(mut self) -> Self {
        self.native_writing_context = None;
        self.writing_workspace_source = None;
        self
    }

    #[cfg(test)]
    pub(crate) fn without_static_writing_compile_sources(mut self) -> Self {
        self.native_writing_context = None;
        self.writing_compile_feedback_source = None;
        #[cfg(target_os = "macos")]
        {
            self.writing_workspace_compile_source = None;
        }
        self
    }

    /// An explicit content-identified native path never falls back to static
    /// trace decisions. The application retains service shutdown ownership.
    #[cfg(any(test, feature = "development-data"))]
    pub(crate) fn with_native_note_trace(
        mut self,
        binding: crate::runtime_plugins::literature_trace::binding::NoteTraceBinding,
    ) -> Self {
        self.native_note_trace = Some(binding);
        self
    }

    pub(crate) fn native_note_trace(
        &self,
    ) -> Option<&crate::runtime_plugins::literature_trace::binding::NoteTraceBinding> {
        self.native_note_trace.as_ref()
    }

    pub(crate) fn open(app: &AppHandle, sources: AppStoreSources) -> Result<Self, StorageError> {
        validate_data_environment(
            &app.config().identifier,
            cfg!(debug_assertions),
            cfg!(feature = "development-data"),
        )?;
        #[cfg(feature = "development-data")]
        let data_directory = crate::storage_environment::development_data_directory_override()?
            .unwrap_or(app.path().app_data_dir()?);
        #[cfg(not(feature = "development-data"))]
        let data_directory = app.path().app_data_dir()?;
        let agent_configuration_directory = app.path().app_config_dir()?;
        Self::open_with_paths(
            AppStorePaths {
                data_directory,
                agent_configuration_directory,
            },
            AppStorePorts::for_app(app.clone(), sources),
        )
    }

    pub(crate) fn connection(&self) -> Result<DatabaseGuard<'_>, StorageError> {
        Ok(self.connection.lock()?)
    }

    /// Invalidate resource views after a durable change; never expose content.
    pub(crate) fn notify_structured_resource_changed(
        &self,
        resource_id: &str,
        paper_id: Option<&str>,
    ) {
        if let Some(events) = &self.events {
            let _ = events.emit(
                "research-assistant:structured-resource-changed",
                serde_json::json!({"resourceId":resource_id,"paperId":paper_id}),
            );
        }
    }

    /// Wake small lifecycle views only after a committed semantic boundary.
    pub(crate) fn notify_agent_activity_changed(&self) {
        if let Some(events) = &self.events {
            let _ = events.emit("research-assistant:agent-activity-changed", ());
        }
    }

    pub(crate) fn notify_agent_subtasks_changed(&self, parent: Option<uuid::Uuid>) {
        if let Some(events) = &self.events {
            let payload = parent.map_or_else(
                || serde_json::json!({}),
                |id| serde_json::json!({"parentConversationId":id}),
            );
            let _ = events.emit("research-assistant:agent-subtasks-changed", payload);
        }
    }

    /// Serialize canonical bibliography transactions and their filesystem projections.
    /// Acquire before the database mutex; never acquire recursively.
    pub(crate) fn bibliography_io(&self) -> Result<MutexGuard<'_, ()>, StorageError> {
        self.bibliography_io
            .lock()
            .map_err(|_| StorageError::DatabaseBusy)
    }

    pub(crate) fn shared_agent_connection(&self) -> SharedDatabase {
        Arc::clone(&self.connection)
    }
    pub(crate) fn writing_delivery_tools(
        &self,
    ) -> &Arc<crate::agent::writing_delivery_tool::DeliveryTools> {
        &self.writing_delivery_tools
    }
    pub(crate) fn text_editors(
        &self,
    ) -> &Arc<crate::agent::text_tools::editor_barrier::EditorBarrier> {
        &self.text_editors
    }

    pub(crate) fn content_directory(&self) -> &Path {
        &self.content_directory
    }

    pub(crate) fn content_read(&self) -> Result<RwLockReadGuard<'_, ()>, StorageError> {
        self.content_io
            .read()
            .map_err(|_| StorageError::DatabaseBusy)
    }

    pub(crate) fn content_write(&self) -> Result<RwLockWriteGuard<'_, ()>, StorageError> {
        self.content_io
            .write()
            .map_err(|_| StorageError::DatabaseBusy)
    }

    pub(crate) fn content_disk_space(&self) -> Result<(u64, u64), StorageError> {
        let stats = nix::sys::statvfs::statvfs(&self.content_directory)
            .map_err(|error| StorageError::DiskSpace(error.to_string()))?;
        let fragment_size = stats.fragment_size();
        Ok((
            u64::from(stats.blocks_available()).saturating_mul(fragment_size),
            u64::from(stats.blocks()).saturating_mul(fragment_size),
        ))
    }

    pub(crate) fn ensure_content_import_capacity(
        &self,
        incoming_bytes: u64,
    ) -> Result<(), StorageError> {
        const SAFETY_MARGIN_BYTES: u64 = 512 * 1024 * 1024;
        let (available, _) = self.content_disk_space()?;
        if available.saturating_sub(incoming_bytes) < SAFETY_MARGIN_BYTES {
            return Err(StorageError::InsufficientDiskSpace);
        }
        Ok(())
    }

    pub(crate) fn manuscript_directory(&self) -> &Path {
        &self.manuscript_directory
    }
    pub(crate) fn manuscript_files(&self, id: &str) -> crate::scoped_text_files::ScopedTextFiles {
        crate::scoped_text_files::ScopedTextFiles::new(&self.manuscript_directory, id)
    }
    pub(crate) fn agent_configuration_directory(&self) -> &Path {
        &self.agent_configuration_directory
    }
    pub(crate) fn agent_workspace_directory(&self) -> &Path {
        &self.agent_workspace_directory
    }

    pub(crate) fn semantic_index_directory(&self) -> &Path {
        &self.semantic_index_directory
    }

    pub(crate) fn writing_cache_directory(&self) -> &Path {
        &self.writing_cache_directory
    }
    pub(crate) fn system_document_approvals(
        &self,
    ) -> &crate::system_documents::approvals::Approvals {
        &self.system_document_approvals
    }

    pub(crate) fn text_patch_boot(&self) -> uuid::Uuid {
        self.text_patch_boot
    }

    #[cfg(test)]
    pub(crate) fn for_test(connection: Connection, content_directory: PathBuf) -> Self {
        Self::for_test_owner(
            Arc::new(Mutex::new(connection)).into_database(),
            content_directory,
        )
    }

    #[cfg(test)]
    pub(crate) fn for_test_database(
        connection: SharedDatabase,
        content_directory: PathBuf,
    ) -> Self {
        Self::for_test_owner(connection, content_directory)
    }

    #[cfg(test)]
    pub(crate) fn set_test_writing_delivery_tools(
        &mut self,
        tools: Arc<crate::agent::writing_delivery_tool::DeliveryTools>,
    ) {
        self.writing_delivery_tools = tools;
    }

    #[cfg(test)]
    pub(crate) fn with_test_paper_metadata_source(
        mut self,
        source: crate::agent::paper_metadata::PaperMetadataSource,
    ) -> Self {
        self.paper_metadata_source = Some(source);
        self
    }

    #[cfg(test)]
    fn for_test_owner(connection: SharedDatabase, content_directory: PathBuf) -> Self {
        let data_directory = content_directory
            .parent()
            .unwrap_or(&content_directory)
            .to_path_buf();
        let sources = crate::core_sources::sources();
        runtime_test_sources::attach(Self {
            events: None,
            docling: Arc::default(),
            connection,
            content_io: Arc::new(RwLock::new(())),
            content_session_retained: Arc::default(),
            bibliography_io: Arc::new(Mutex::new(())),
            bibliography_publications: Arc::default(),
            // Tests select the same real providers as production composition.
            writing_context_source: sources.writing_context_source,
            writing_resource_source: Some(crate::agent::writing_resources::test_sources::source()),
            paper_metadata_source: sources.paper_metadata_source,
            retrieval_trace_source: sources.retrieval_trace_source,
            retrieval_preview_source: sources.retrieval_preview_source.map(Arc::new),
            semantic_index_source: sources.semantic_index_source.map(Arc::new),
            native_note_trace: None,
            native_writing_context: None,
            paper_portrait_source: sources.paper_portrait_source,
            context_catalog_source: sources.context_catalog_source,
            generation_policy_source: sources.generation_policy_source,
            paper_generation_source: sources.paper_generation_source,
            paper_reading_source: sources.paper_reading_source,
            paper_analysis_source: sources.paper_analysis_source,
            writing_compile_feedback_source: sources.writing_compile_feedback_source,
            #[cfg(target_os = "macos")]
            writing_workspace_compile_source: sources.writing_workspace_compile_source,
            #[cfg(unix)]
            writing_workspace_source: sources.writing_workspace_source,
            content_directory,
            manuscript_directory: data_directory.join("manuscripts"),
            semantic_index_directory: data_directory.join("semantic-indexes"),
            writing_cache_directory: data_directory.join("cache").join("writing"),
            agent_configuration_directory: data_directory.join("test-config"),
            agent_workspace_directory: data_directory.join("agent-workspaces"),
            text_editors: Arc::default(),
            writing_delivery_tools: Arc::default(),
            text_patch_boot: uuid::Uuid::new_v4(),
            system_document_approvals: Arc::default(),
        })
    }
}

/// Test-only migration entry using the same Core source selection as startup.
#[cfg(test)]
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), StorageError> {
    let sources = crate::core_sources::sources();
    migrate_with_catalog_functions(
        connection,
        sources.paper_analysis_source,
        sources.candidate_recovery_source,
    )
}

#[cfg(test)]
pub(crate) fn migrate_with_catalog_functions(
    connection: &mut Connection,
    paper_analysis_source: Option<crate::ai::analysis_source::PaperAnalysisSource>,
    candidate_recovery_source: Option<CandidateRecoverySource>,
) -> Result<(), StorageError> {
    migrate_with_recovery_sources(
        connection,
        paper_analysis_source,
        candidate_recovery_source,
        None,
    )
}

fn migrate_with_recovery_sources(
    connection: &mut Connection,
    paper_analysis_source: Option<crate::ai::analysis_source::PaperAnalysisSource>,
    candidate_recovery_source: Option<CandidateRecoverySource>,
    native: Option<&crate::runtime_plugins::literature_trace::binding::NoteTraceBinding>,
) -> Result<(), StorageError> {
    crate::notes::register_projection_functions(connection)?;
    schema_compat::register_functions(connection)?;
    crate::data_interface::operation_identity::register(connection)?;
    crate::data_interface::value_functions::register(connection)?;
    connection.execute_batch(
        "pragma foreign_keys = on;
         pragma journal_mode = wal;
         create table if not exists schema_migrations (
             version integer primary key,
             applied_at text not null
         );",
    )?;

    for &(version, sql) in MIGRATIONS {
        apply_migration(connection, version, sql)?;
    }
    backfill_migration_checksums(connection)?;
    validate_ai_candidate_schema(connection)?;
    let native_recovery = if let Some(binding) = native {
        Some(tauri::async_runtime::block_on(
            binding
                .service
                .recover_startup_candidates(connection, &binding.package_key),
        )?)
    } else {
        None
    };
    if native_recovery.is_none()
        && let Some(recover) = candidate_recovery_source
    {
        recover(connection)?;
    }
    connection.execute(
        "update import_jobs set state = 'interrupted', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where state in ('queued', 'running')",
        [],
    )?;
    connection.execute(
        "update full_text_index_jobs set state = 'interrupted', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where state = 'running'",
        [],
    )?;
    connection.execute(
        "update manuscript_compile_runs set status = 'interrupted',
             finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where status in ('queued', 'running')",
        [],
    )?;
    if candidate_recovery_source.is_some() || native_recovery.is_some() {
        connection.execute_batch(
            "update ai_run_stages
         set status = 'failed', error_code = 'interrupted',
             error_message = '应用上次退出时任务仍在运行；请检查后显式重试',
             finished_at = coalesce(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         where status in ('queued', 'running');
         update ai_runs
         set status = 'failed', error_code = 'interrupted',
             error_message = '应用上次退出时任务仍在运行；不会自动重新发送数据',
             finished_at = coalesce(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where status in (
             'queued', 'resolving_scope', 'retrieving', 'extracting_evidence',
             'generating', 'validating'
         );",
        )?;
    } else {
        // Preserve the domain recovery predicate and original causes while its
        // owner is absent. Reopen is idempotent; restoration runs the original
        // callback above before the ordinary terminalization path.
        connection.execute_batch(
            "update ai_runs set error_code='capability_unavailable',
                 error_message=coalesce(error_message,'当前构建未加载文献管理；原操作已保留，恢复该插件后继续核对'),
                 updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             where error_code is null and status in (
                 'queued','resolving_scope','retrieving','extracting_evidence','generating','validating'
             );
             update ai_run_stages set error_code='capability_unavailable',
                 error_message=coalesce(error_message,'当前构建未加载文献管理；原阶段已保留')
             where error_code is null and status in ('queued','running');",
        )?;
    }
    if let Some(resources) = native_recovery {
        resources.interrupt(connection)?;
    } else if let Some(source) = paper_analysis_source {
        (source.interrupt)(connection)?;
    }
    connection.execute(
        "update content_gc_state set job_state = 'interrupted',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where job_state = 'running'",
        [],
    )?;
    connection.execute_batch(
        "update embedding_outbox_jobs
         set status = 'interrupted', lease_owner = null, lease_expires_at = null,
             error_code = 'interrupted',
             finished_at = coalesce(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         where status = 'running';
         update embedding_records
         set status = 'failed', error_code = 'interrupted',
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where status = 'running';
         update embedding_reconciliation_runs
         set status = 'interrupted',
             finished_at = coalesce(finished_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             error_code = 'interrupted'
         where status = 'running';",
    )?;
    Ok(())
}

fn apply_migration(
    connection: &mut Connection,
    version: i64,
    statements: &str,
) -> Result<(), StorageError> {
    let exists = connection.query_row(
        "select exists(select 1 from schema_migrations where version = ?1)",
        [version],
        |row| row.get::<_, bool>(0),
    )?;

    if exists {
        verify_recorded_migration_checksum(connection, version, statements)?;
        return Ok(());
    }

    let requires_foreign_keys_disabled = MIGRATIONS_WITH_FOREIGN_KEYS_DISABLED.contains(&version);
    if requires_foreign_keys_disabled {
        connection.execute_batch("pragma foreign_keys = off;")?;
    }
    let migration_result = (|| -> Result<(), StorageError> {
        let transaction = connection.transaction()?;
        transaction.execute_batch(statements)?;
        if version == 212 {
            crate::agent::repository::search_visibility::backfill_in(&transaction)?;
        }
        if requires_foreign_keys_disabled {
            let violation_count = transaction
                .prepare("pragma foreign_key_check")?
                .query_map([], |_| Ok(()))?
                .count();
            if violation_count > 0 {
                return Err(StorageError::MigrationIntegrity(format!(
                    "迁移 {version} 后发现 {violation_count} 个外键错误"
                )));
            }
        }
        transaction.execute(
            "insert into schema_migrations (version, applied_at)
             values (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            [version],
        )?;
        record_migration_checksum(&transaction, version, statements)?;
        transaction.commit()?;
        Ok(())
    })();
    if requires_foreign_keys_disabled {
        connection.execute_batch("pragma foreign_keys = on;")?;
    }
    migration_result?;
    Ok(())
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "concept_lifecycle_migration_tests.rs"]
mod concept_lifecycle_migration_tests;

#[cfg(test)]
#[path = "managed_bibliography_migration_tests.rs"]
mod managed_bibliography_migration_tests;

#[cfg(test)]
#[path = "storage_rollback_search_tests.rs"]
mod rollback_search_tests;

#[cfg(test)]
#[path = "storage_writing_resources_tests.rs"]
mod writing_resources_tests;
