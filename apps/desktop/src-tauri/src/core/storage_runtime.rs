//! One paths/ports factory for desktop and isolated development runtimes.
use super::*;
use crate::{
    agent::{text_tools::editor_barrier::EditorBarrier, writing_delivery_tool::DeliveryTools},
    runtime_events::RuntimeEvents,
};

pub(crate) struct AppStorePaths {
    pub data_directory: PathBuf,
    pub agent_configuration_directory: PathBuf,
}

/// Typed bootstrap values only; this neither resolves providers nor owns a runtime.
#[derive(Default)]
pub(crate) struct AppStoreSources {
    pub native_note_trace:
        Option<crate::runtime_plugins::literature_trace::binding::NoteTraceBinding>,
    pub writing_context_source: Option<crate::agent::writing_context::WritingContextSource>,
    pub writing_resource_source: Option<crate::agent::writing_resources::WritingResourceSource>,
    pub paper_metadata_source: Option<crate::agent::paper_metadata::PaperMetadataSource>,
    pub retrieval_trace_source: Option<crate::agent::retrieval_context::RetrievalTraceSource>,
    pub retrieval_preview_source:
        Option<crate::runtime_plugins::literature_retrieval::source::RetrievalPreviewSource>,
    pub semantic_index_source:
        Option<crate::runtime_plugins::literature_index::source::SemanticIndexSource>,
    pub paper_portrait_source: Option<crate::agent::paper_portrait::PaperPortraitSource>,
    pub context_catalog_source: crate::agent::context_catalog::ContextCatalogSource,
    pub generation_policy_source: Option<crate::ai::generation_policy::GenerationPolicySource>,
    pub paper_generation_source:
        Option<crate::ai::paper_generation_admission::PaperGenerationSource>,
    pub paper_reading_source: Option<crate::ai::paper_reading_source::PaperReadingSource>,
    pub paper_analysis_source: Option<crate::ai::analysis_source::PaperAnalysisSource>,
    pub candidate_recovery_source: Option<CandidateRecoverySource>,
    pub writing_compile_feedback_source:
        Option<crate::agent::compile_feedback::WritingCompileFeedbackSource>,
    #[cfg(unix)]
    pub writing_workspace_source: Option<crate::agent::writing_workspace::WritingWorkspaceSource>,
    #[cfg(target_os = "macos")]
    pub writing_workspace_compile_source:
        Option<crate::agent::workspace_compile::WritingWorkspaceCompileSource>,
}

pub(crate) struct AppStorePorts {
    pub native_note_trace:
        Option<crate::runtime_plugins::literature_trace::binding::NoteTraceBinding>,
    pub events: RuntimeEvents,
    pub text_editors: Arc<EditorBarrier>,
    pub writing_delivery_tools: Arc<DeliveryTools>,
    pub writing_context_source: Option<crate::agent::writing_context::WritingContextSource>,
    pub writing_resource_source: Option<crate::agent::writing_resources::WritingResourceSource>,
    pub paper_metadata_source: Option<crate::agent::paper_metadata::PaperMetadataSource>,
    pub retrieval_trace_source: Option<crate::agent::retrieval_context::RetrievalTraceSource>,
    pub retrieval_preview_source:
        Option<crate::runtime_plugins::literature_retrieval::source::RetrievalPreviewSource>,
    pub semantic_index_source:
        Option<crate::runtime_plugins::literature_index::source::SemanticIndexSource>,
    pub paper_portrait_source: Option<crate::agent::paper_portrait::PaperPortraitSource>,
    pub context_catalog_source: crate::agent::context_catalog::ContextCatalogSource,
    pub generation_policy_source: Option<crate::ai::generation_policy::GenerationPolicySource>,
    pub paper_generation_source:
        Option<crate::ai::paper_generation_admission::PaperGenerationSource>,
    pub paper_reading_source: Option<crate::ai::paper_reading_source::PaperReadingSource>,
    pub paper_analysis_source: Option<crate::ai::analysis_source::PaperAnalysisSource>,
    pub candidate_recovery_source: Option<CandidateRecoverySource>,
    pub writing_compile_feedback_source:
        Option<crate::agent::compile_feedback::WritingCompileFeedbackSource>,
    #[cfg(unix)]
    pub writing_workspace_source: Option<crate::agent::writing_workspace::WritingWorkspaceSource>,
    #[cfg(target_os = "macos")]
    pub writing_workspace_compile_source:
        Option<crate::agent::workspace_compile::WritingWorkspaceCompileSource>,
}

impl AppStorePorts {
    pub(super) fn for_app(app: AppHandle, sources: AppStoreSources) -> Self {
        Self {
            native_note_trace: sources.native_note_trace,
            events: RuntimeEvents::for_app(app.clone()),
            text_editors: Arc::new(EditorBarrier::for_app(app.clone())),
            writing_delivery_tools: Arc::new(DeliveryTools::for_app(app)),
            writing_context_source: sources.writing_context_source,
            writing_resource_source: sources.writing_resource_source,
            paper_metadata_source: sources.paper_metadata_source,
            retrieval_trace_source: sources.retrieval_trace_source,
            retrieval_preview_source: sources.retrieval_preview_source,
            semantic_index_source: sources.semantic_index_source,
            paper_portrait_source: sources.paper_portrait_source,
            context_catalog_source: sources.context_catalog_source,
            generation_policy_source: sources.generation_policy_source,
            paper_generation_source: sources.paper_generation_source,
            paper_reading_source: sources.paper_reading_source,
            paper_analysis_source: sources.paper_analysis_source,
            candidate_recovery_source: sources.candidate_recovery_source,
            writing_compile_feedback_source: sources.writing_compile_feedback_source,
            #[cfg(unix)]
            writing_workspace_source: sources.writing_workspace_source,
            #[cfg(target_os = "macos")]
            writing_workspace_compile_source: sources.writing_workspace_compile_source,
        }
    }

    #[cfg(any(test, all(debug_assertions, feature = "development-data")))]
    pub(crate) fn with_events(events: RuntimeEvents, sources: AppStoreSources) -> Self {
        Self {
            native_note_trace: sources.native_note_trace,
            text_editors: Arc::new(EditorBarrier::for_events(events.clone())),
            writing_delivery_tools: Arc::new(DeliveryTools::for_events(events.clone())),
            events,
            writing_context_source: sources.writing_context_source,
            writing_resource_source: sources.writing_resource_source,
            paper_metadata_source: sources.paper_metadata_source,
            retrieval_trace_source: sources.retrieval_trace_source,
            retrieval_preview_source: sources.retrieval_preview_source,
            semantic_index_source: sources.semantic_index_source,
            paper_portrait_source: sources.paper_portrait_source,
            context_catalog_source: sources.context_catalog_source,
            generation_policy_source: sources.generation_policy_source,
            paper_generation_source: sources.paper_generation_source,
            paper_reading_source: sources.paper_reading_source,
            paper_analysis_source: sources.paper_analysis_source,
            candidate_recovery_source: sources.candidate_recovery_source,
            writing_compile_feedback_source: sources.writing_compile_feedback_source,
            #[cfg(unix)]
            writing_workspace_source: sources.writing_workspace_source,
            #[cfg(target_os = "macos")]
            writing_workspace_compile_source: sources.writing_workspace_compile_source,
        }
    }
}

impl AppStore {
    pub(crate) fn require_generation_policy_source(
        &self,
    ) -> Result<crate::ai::generation_policy::GenerationPolicySource, crate::ai::error::AiError>
    {
        self.generation_policy_source
            .ok_or(crate::ai::error::AiError::CapabilityUnavailable(
                "literature_library.generation_policy_source",
            ))
    }

    pub(crate) fn require_paper_generation_source(
        &self,
    ) -> Result<
        crate::ai::paper_generation_admission::PaperGenerationSource,
        crate::ai::error::AiError,
    > {
        self.paper_generation_source
            .ok_or(crate::ai::error::AiError::CapabilityUnavailable(
                "literature_library.paper_generation_source",
            ))
    }

    pub(crate) fn require_paper_reading_source(
        &self,
    ) -> Result<crate::ai::paper_reading_source::PaperReadingSource, crate::ai::error::AiError>
    {
        self.paper_reading_source
            .ok_or(crate::ai::error::AiError::CapabilityUnavailable(
                "literature_library.paper_reading_source",
            ))
    }

    pub(crate) fn require_paper_analysis_source(
        &self,
    ) -> Result<crate::ai::analysis_source::PaperAnalysisSource, crate::ai::error::AiError> {
        self.paper_analysis_source
            .ok_or(crate::ai::error::AiError::CapabilityUnavailable(
                "literature_library.paper_analysis_source",
            ))
    }

    pub(crate) fn semantic_index_source(
        &self,
    ) -> Option<crate::runtime_plugins::literature_index::source::SemanticIndexSource> {
        self.semantic_index_source.as_deref().copied()
    }
    pub(crate) fn retrieval_preview_source(
        &self,
    ) -> Option<crate::runtime_plugins::literature_retrieval::source::RetrievalPreviewSource> {
        self.retrieval_preview_source.as_deref().copied()
    }
    pub(crate) fn retrieval_trace_source(
        &self,
    ) -> Option<crate::agent::retrieval_context::RetrievalTraceSource> {
        self.retrieval_trace_source
    }

    pub(crate) fn paper_portrait_source(
        &self,
    ) -> Option<crate::agent::paper_portrait::PaperPortraitSource> {
        self.paper_portrait_source
    }

    pub(crate) fn context_catalog_source(
        &self,
    ) -> crate::agent::context_catalog::ContextCatalogSource {
        self.context_catalog_source
    }

    pub(crate) fn generation_policy_source(
        &self,
    ) -> Option<crate::ai::generation_policy::GenerationPolicySource> {
        self.generation_policy_source
    }

    pub(crate) fn paper_analysis_source(
        &self,
    ) -> Option<crate::ai::analysis_source::PaperAnalysisSource> {
        self.paper_analysis_source
    }

    pub(crate) fn paper_reading_source(
        &self,
    ) -> Option<crate::ai::paper_reading_source::PaperReadingSource> {
        self.paper_reading_source
    }

    #[cfg(test)]
    pub(crate) fn paper_generation_source(
        &self,
    ) -> Option<crate::ai::paper_generation_admission::PaperGenerationSource> {
        self.paper_generation_source
    }

    pub(crate) fn writing_compile_feedback_source(
        &self,
    ) -> Option<crate::agent::compile_feedback::WritingCompileFeedbackSource> {
        self.writing_compile_feedback_source
    }

    pub(crate) fn paper_metadata_source(
        &self,
    ) -> Option<crate::agent::paper_metadata::PaperMetadataSource> {
        self.paper_metadata_source
    }

    pub(crate) fn writing_resource_source(
        &self,
    ) -> Option<crate::agent::writing_resources::WritingResourceSource> {
        self.writing_resource_source
    }

    pub(crate) fn writing_source_directory(
        &self,
    ) -> Option<crate::agent::writing_resources::SourceDirectory> {
        if self.native_writing_context.is_some() {
            Some(crate::agent::writing_resources::source_directory)
        } else {
            self.writing_resource_source
                .map(|source| source.source_directory)
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn writing_workspace_compile_source(
        &self,
    ) -> Option<crate::agent::workspace_compile::WritingWorkspaceCompileSource> {
        self.writing_workspace_compile_source
    }

    /// Internal repository callback: borrow the caller's snapshot without another
    /// connection, transaction, worker or domain runtime owner.
    pub(crate) fn materialize_writing_context_in(
        &self,
        db: &Connection,
        request: crate::agent::WritingContextRequest,
    ) -> Result<Vec<research_agent_core::submission::ContextText>, crate::agent::WritingContextError>
    {
        let source = self
            .writing_context_source
            .ok_or(crate::agent::WritingContextError::Unavailable)?;
        (source.selected)(db, self.manuscript_directory(), request)
    }

    pub(crate) fn materialize_current_writing_file_in(
        &self,
        db: &Connection,
        manuscript: uuid::Uuid,
        path: &str,
    ) -> Result<Vec<research_agent_core::submission::ContextText>, crate::agent::WritingContextError>
    {
        let source = self
            .writing_context_source
            .ok_or(crate::agent::WritingContextError::Unavailable)?;
        (source.current_file)(db, self.manuscript_directory(), manuscript, path)
    }

    pub(crate) fn open_with_paths(
        paths: AppStorePaths,
        ports: AppStorePorts,
    ) -> Result<Self, StorageError> {
        let AppStorePaths {
            data_directory,
            agent_configuration_directory,
        } = paths;
        let content_directory = data_directory.join("objects");
        let manuscript_directory = data_directory.join("manuscripts");
        let semantic_index_directory = data_directory.join("semantic-indexes");
        let writing_cache_directory = data_directory.join("cache").join("writing");
        for directory in [
            &content_directory,
            &manuscript_directory,
            &semantic_index_directory,
            &writing_cache_directory,
            &agent_configuration_directory,
        ] {
            fs::create_dir_all(directory)?;
        }
        let database_path = data_directory.join("research-assistant.sqlite3");
        let log_path = data_directory.join("data-log");
        let database_existed = database_path.exists();
        if !database_existed && log_path.exists() {
            return Err(crate::data_interface::DataError::Pair(
                "SQL database missing beside existing log",
            )
            .into());
        }
        let connection = Connection::open(&database_path)?;
        let locked_log = Database::lock_existing_pair(&connection, &log_path)?;
        if database_existed {
            backup_before_pending_migrations_locked(
                &connection,
                &data_directory.join("backups"),
                locked_log.as_ref(),
            )?;
        }
        let (mut connection, locked_log) =
            super::startup::prepare(connection, &log_path, locked_log)?;
        migrate_with_recovery_sources(
            &mut connection,
            ports.paper_analysis_source,
            ports.candidate_recovery_source,
            ports.native_note_trace.as_ref(),
        )?;
        let connection = Database::open_with_log(connection, &log_path, locked_log)?;
        Ok(Self {
            events: Some(ports.events),
            docling: Arc::default(),
            connection,
            content_io: Arc::new(RwLock::new(())),
            content_session_retained: Arc::default(),
            bibliography_io: Arc::new(Mutex::new(())),
            bibliography_publications: Arc::default(),
            writing_context_source: ports.writing_context_source,
            writing_resource_source: ports.writing_resource_source,
            paper_metadata_source: ports.paper_metadata_source,
            retrieval_trace_source: ports.retrieval_trace_source,
            retrieval_preview_source: ports.retrieval_preview_source.map(Arc::new),
            semantic_index_source: ports.semantic_index_source.map(Arc::new),
            native_note_trace: ports.native_note_trace,
            native_writing_context: None,
            paper_portrait_source: ports.paper_portrait_source,
            context_catalog_source: ports.context_catalog_source,
            generation_policy_source: ports.generation_policy_source,
            paper_generation_source: ports.paper_generation_source,
            paper_reading_source: ports.paper_reading_source,
            paper_analysis_source: ports.paper_analysis_source,
            writing_compile_feedback_source: ports.writing_compile_feedback_source,
            #[cfg(unix)]
            writing_workspace_source: ports.writing_workspace_source,
            #[cfg(target_os = "macos")]
            writing_workspace_compile_source: ports.writing_workspace_compile_source,
            content_directory,
            manuscript_directory,
            semantic_index_directory,
            writing_cache_directory,
            agent_configuration_directory,
            agent_workspace_directory: data_directory.join("agent-workspaces"),
            text_editors: ports.text_editors,
            text_patch_boot: uuid::Uuid::new_v4(),
            system_document_approvals: Arc::default(),
            writing_delivery_tools: ports.writing_delivery_tools,
        })
    }
}
