//! Fixed original index stages over owned Core state and finite Native rules.
use super::{
    decisions::{Cache, Decided, IndexRun},
    failure::Error,
    generation_repository as generations,
    model::{PendingEmbeddingJob, SemanticIndexError},
    reconciliation_repository as reconciliation, repository,
    resources::IndexResources,
};
use crate::{
    ai::{EmbeddingAccess, PreparedEmbedding},
    data_interface::guarded_transaction,
    runtime_plugins::literature_trace::service::LiteratureTraceService,
    storage::AppStore,
};
use research_workbench_native_sdk::semantic_index::{
    EmbeddingJobSummary, IndexGenerationSummary, SemanticIndexStatus,
};
use research_workbench_native_sdk::semantic_index::{
    ReconciliationCounts, ReconciliationGeneration, ReconciliationRecord, ReconciliationSummary,
};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum Operation {
    Status,
    Rebuild(String),
    Job(String),
    Retry(String),
    Prepare {
        id: String,
        embedding: EmbeddingAccess,
    },
    Running {
        id: String,
        lease: &'static str,
    },
    Ready {
        job: Box<PendingEmbeddingJob>,
        version: u64,
        norm: f32,
    },
    Interrupt(String),
    Failed {
        id: String,
        code: &'static str,
        canceled: bool,
    },
    BeginReconciliation,
    ReconciliationTargets,
    MarkStale(ReconciliationRecord),
    FinishReconciliation {
        id: String,
        status: &'static str,
        counts: ReconciliationCounts,
        error: Option<&'static str>,
    },
}
pub(crate) enum Output {
    Status(SemanticIndexStatus),
    Generation(IndexGenerationSummary),
    Job(EmbeddingJobSummary),
    Retry(EmbeddingJobSummary, bool),
    Prepared(Box<(PendingEmbeddingJob, PreparedEmbedding)>),
    Running(Box<PendingEmbeddingJob>),
    Complete,
    RunId(String),
    Targets(Vec<ReconciliationGeneration>),
    Reconciled(ReconciliationSummary),
}

pub(super) async fn run(
    owner: &LiteratureTraceService,
    store: AppStore,
    key: &str,
    resources: Arc<IndexResources>,
    operation: Operation,
) -> Result<Output, Error> {
    let mut cache = Cache::default();
    loop {
        let (next, result) = attempt(
            store.clone(),
            Arc::clone(&resources),
            operation.clone(),
            cache,
        )
        .await?;
        cache = next;
        match result {
            Err(Error::Need(need)) => {
                let outcome = if matches!(
                    operation,
                    Operation::Retry(_) | Operation::Running { .. } | Operation::Ready { .. }
                ) {
                    crate::hbs_storage::boundary::HbsOutcome::RolledBack
                } else {
                    crate::hbs_storage::boundary::HbsOutcome::NotStarted
                };
                let (decision, origin) = owner
                    .semantic_index_rule(key, need.input.clone())
                    .await
                    .map_err(|error| Error::Native { error, outcome })?;
                cache.0.push(Decided {
                    need: *need,
                    decision,
                    origin,
                });
            }
            result => return result,
        }
    }
}

async fn attempt(
    store: AppStore,
    resources: Arc<IndexResources>,
    operation: Operation,
    cache: Cache,
) -> Result<(Cache, Result<Output, Error>), Error> {
    tokio::task::spawn_blocking(move || {
        let mut db = store.connection()?;
        let result = guarded_transaction::run_mut(&mut db, |db| {
            let run = IndexRun::new(&resources, &cache);
            let result = if db.is_autocommit() {
                step(&run, db, &operation)
            } else {
                Err(Error::Unsettled)
            };
            let unknown = !db.is_autocommit() || result.as_ref().is_err_and(Error::needs_recovery);
            (result, unknown)
        });
        Ok((cache, result))
    })
    .await
    .map_err(|_| Error::Unsettled)?
}

fn pending(
    run: &IndexRun<'_>,
    db: &rusqlite::Connection,
    id: &str,
) -> Result<PendingEmbeddingJob, Error> {
    repository::load_job(run, db, id)?
        .ok_or_else(|| SemanticIndexError::InvalidInput("找不到 embedding job").into())
}
fn step(
    run: &IndexRun<'_>,
    db: &mut rusqlite::Connection,
    operation: &Operation,
) -> Result<Output, Error> {
    match operation {
        Operation::Status => repository::get_status(run, db).map(Output::Status),
        Operation::Rebuild(id) => {
            generations::create_rebuild_generation(run, db, id).map(Output::Generation)
        }
        Operation::Job(id) => repository::get_job_summary(run, db, id).map(Output::Job),
        Operation::Retry(id) => repository::queue_retry(run, db, id)
            .map(|(summary, start)| Output::Retry(summary, start)),
        Operation::Prepare { id, embedding } => {
            let job = pending(run, db, id)?;
            let prepared = embedding
                .prepare_in(db)
                .map_err(|_| SemanticIndexError::Provider("configuration"))?;
            Ok(Output::Prepared(Box::new((job, prepared))))
        }
        Operation::Running { id, lease } => {
            let job = pending(run, db, id)?;
            repository::mark_running(run, db, id, lease)?;
            Ok(Output::Running(Box::new(job)))
        }
        Operation::Ready { job, version, norm } => {
            repository::mark_ready(run, db, job, *version, *norm)?;
            Ok(Output::Complete)
        }
        Operation::Interrupt(id) => {
            repository::mark_interrupted(run, db, id)?;
            Ok(Output::Complete)
        }
        Operation::Failed { id, code, canceled } => {
            if *canceled {
                repository::mark_canceled(run, db, id)?;
            } else {
                repository::mark_failed(run, db, id, code)?;
            }
            Ok(Output::Complete)
        }
        Operation::BeginReconciliation => reconciliation::begin_run(run, db).map(Output::RunId),
        Operation::ReconciliationTargets => {
            reconciliation::load_generations(run, db).map(Output::Targets)
        }
        Operation::MarkStale(record) => {
            reconciliation::mark_stale(run, db, record)?;
            Ok(Output::Complete)
        }
        Operation::FinishReconciliation {
            id,
            status,
            counts,
            error,
        } => {
            reconciliation::finish_run(run, db, id, status, *counts, *error)?;
            reconciliation::get_summary(run, db, id).map(Output::Reconciled)
        }
    }
}
