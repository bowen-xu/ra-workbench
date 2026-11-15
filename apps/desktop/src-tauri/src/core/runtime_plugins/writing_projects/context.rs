//! Request-owned writing source decisions inside the original SQL preparation.
#[path = "context/catalog.rs"]
mod catalog;
#[path = "context/reader.rs"]
mod reader;
#[path = "context/resources.rs"]
mod resources;
#[path = "context/response.rs"]
mod response;
#[path = "context/scope.rs"]
mod scope;
#[cfg(unix)]
#[path = "context/workspace.rs"]
mod workspace;
use super::failure;
use crate::{
    agent::{
        repository::{
            WorkerRetention,
            input_queue::queued_compaction::{PrepareIntent, PrepareSource, QueuedCompactionState},
        },
        writing_context::ContextNeed,
        writing_submission::{WritingStorageError, WritingSubmissionError, prepare_worker_failure},
    },
    ai::structured_resource_validation::Flow,
    data_interface::{
        QueuePreviewPrepareIntent, QueuePreviewPrepareSource, QueuePreviewPrepared, SharedDatabase,
        SqlImmediateFailure, WritingPrepareIntent, WritingPrepareSource,
    },
    runtime_plugins::{
        RuntimePluginCatalog,
        native::{NativePreviewService, NativeProfile, package},
    },
};
use reader::Reader;
use resources::Resources;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Binding {
    pub service: Arc<NativePreviewService>,
    pub catalog: Arc<RuntimePluginCatalog>,
    pub package_key: String,
}
impl Binding {
    pub(crate) async fn prepare(
        &self,
        database: SharedDatabase,
        intent: WritingPrepareIntent,
        source: WritingPrepareSource,
        retention: WorkerRetention,
    ) -> Result<research_agent_core::submission::SubmissionRecord, WritingSubmissionError> {
        let operation = intent.operation();
        let scope_required = needs_scope(&source.request.resources);
        self.run(
            database,
            operation,
            retention,
            scope_required,
            move |db, reader| db.prepare_writing_context(&intent, source.clone(), reader),
        )
        .await
    }
    pub(crate) async fn preview(
        &self,
        database: SharedDatabase,
        intent: QueuePreviewPrepareIntent,
        source: QueuePreviewPrepareSource,
        retention: WorkerRetention,
    ) -> Result<QueuePreviewPrepared, WritingSubmissionError> {
        let scope_required = needs_scope(&source.request.resources);
        self.run(
            database,
            intent.request_id,
            retention,
            scope_required,
            move |db, reader| db.prepare_queue_preview_context(&intent, source.clone(), reader),
        )
        .await
    }
    pub(crate) async fn compact(
        &self,
        database: SharedDatabase,
        intent: PrepareIntent,
        source: PrepareSource,
        retention: WorkerRetention,
    ) -> Result<QueuedCompactionState, WritingSubmissionError> {
        let scope_required = needs_scope(&source.request.resources);
        self.run(
            database,
            intent.request_id,
            retention,
            scope_required,
            move |db, reader| db.prepare_queued_compaction_context(&intent, source.clone(), reader),
        )
        .await
    }
    async fn run<T, F>(
        &self,
        database: SharedDatabase,
        operation: uuid::Uuid,
        retention: WorkerRetention,
        scope_required: bool,
        attempt: F,
    ) -> Result<T, WritingSubmissionError>
    where
        T: Send + 'static,
        F: Fn(&SharedDatabase, &mut Reader) -> Result<T, Flow<SqlImmediateFailure, ContextNeed>>
            + Send
            + Sync
            + 'static,
    {
        let binding = self.clone();
        tokio::spawn(async move {
            binding
                .run_owned(
                    database,
                    operation,
                    retention,
                    scope_required,
                    Arc::new(attempt),
                )
                .await
        })
        .await
        .map_err(|_| WritingSubmissionError::from(prepare_worker_failure(operation, true)))?
    }
    async fn run_owned<T, F>(
        self,
        database: SharedDatabase,
        operation: uuid::Uuid,
        retention: WorkerRetention,
        scope_required: bool,
        attempt: Arc<F>,
    ) -> Result<T, WritingSubmissionError>
    where
        T: Send + 'static,
        F: Fn(&SharedDatabase, &mut Reader) -> Result<T, Flow<SqlImmediateFailure, ContextNeed>>
            + Send
            + Sync
            + 'static,
    {
        let _lifetime = self.service.project_operation(&self.package_key).await?;
        let selected = self.package_key.clone();
        let catalog = Arc::clone(&self.catalog);
        let (resources, scope) = tokio::task::spawn_blocking(move || {
            let package = package::accept(&catalog, &selected, NativeProfile::WritingProjects)?;
            let resources = Resources::load(&catalog, &package)
                .map_err(|error| failure(error.code, error.message, &selected))?;
            let scope = scope_required.then(|| scope::ScopeResources::load(&catalog, &package));
            Ok::<_, Box<crate::runtime_plugins::native::NativeHostError>>((resources, scope))
        })
        .await
        .map_err(|_| WritingSubmissionError::from(prepare_worker_failure(operation, false)))??;
        let mut reader = Reader::new(resources, self.package_key.clone()).with_scope(scope);
        loop {
            let database = Arc::clone(&database);
            let attempt = Arc::clone(&attempt);
            let result = retention
                .spawn(move || {
                    reader.begin();
                    let result = attempt(&database, &mut reader);
                    (reader, result)
                })
                .await
                .map_err(|_| {
                    WritingSubmissionError::from(prepare_worker_failure(operation, true))
                })?;
            reader = result.0;
            match result.1 {
                Ok(record) => return Ok(record),
                Err(Flow::Need(input)) => {
                    // The typed SQL owner confirmed rollback before returning Need.
                    // Both the SQL guard and admission have been released here.
                    let decided = self
                        .service
                        .project_decision(self.package_key.clone(), *input)
                        .await
                        .map_err(|mut error| {
                            error.operation_id = Some(operation.to_string());
                            WritingSubmissionError::from(error)
                        })?;
                    reader.resolve(decided)?;
                }
                Err(Flow::Fail(failure)) => {
                    let rejection = reader.rejection(&failure.cause);
                    return Err(WritingStorageError::source_failure(failure, rejection));
                }
            }
        }
    }
}

fn needs_scope(resources: &[crate::agent::context_session::sources::Resource]) -> bool {
    resources.iter().any(|resource| {
        matches!(
            resource,
            crate::agent::context_session::sources::Resource::Manuscript { .. }
        )
    })
}

#[path = "context/current_file.rs"]
mod current_file;

#[path = "context/independent.rs"]
mod independent;
