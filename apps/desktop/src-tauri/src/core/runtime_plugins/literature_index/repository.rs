use research_workbench_native_sdk::semantic_index::{
    EmbeddingJobSummary, SemanticIndexSpaceStatus, SemanticIndexStatus,
    rules::{IndexDecisionV1, IndexInputV1},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use super::{
    decisions::IndexRun,
    failure::{Error, deferred},
    model::SemanticIndexError,
    resources::IndexSql,
};

pub(crate) use research_workbench_native_sdk::semantic_index::PendingEmbeddingJob;

pub(crate) fn load_job(
    run: &IndexRun<'_>,
    connection: &Connection,
    job_id: &str,
) -> Result<Option<PendingEmbeddingJob>, Error> {
    connection
        .query_row(run.sql(IndexSql::RepositoryLoadJob1), [job_id], |row| {
            run.row(row, |row| {
                research_workbench_native_sdk::semantic_index::decode_pending_job(
                    |index| row.get::<_, String>(index),
                    |index| row.get::<_, i64>(index),
                    |message| to_sql_error(SemanticIndexError::InvalidInput(message)),
                )
            })
        })
        .optional()
        .map_err(Error::from)
}

pub(crate) fn queue_retry(
    run: &IndexRun<'_>,
    connection: &mut Connection,
    representation_id: &str,
) -> Result<(EmbeddingJobSummary, bool), Error> {
    deferred(connection, |transaction| {
        let (record_id, record_status) = transaction
            .query_row(
                run.sql(IndexSql::RepositoryQueueRetry1),
                [representation_id],
                |row| {
                    run.row(row, |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                },
            )
            .optional()?
            .ok_or(SemanticIndexError::InvalidInput(
                "找不到可重试的 semantic representation",
            ))?;
        let latest = latest_job(run, transaction, &record_id)?;
        match run.decide(IndexInputV1::Retry {
            record_status,
            latest: latest.clone(),
        })? {
            IndexDecisionV1::ReuseJob {
                summary,
                should_start,
            } if latest.as_ref() == Some(&summary) => return Ok((summary, should_start)),
            IndexDecisionV1::QueueJob => {}
            _ => return Err(Error::mismatch()),
        }
        let attempt = transaction.query_row(
            run.sql(IndexSql::RepositoryQueueRetry2),
            [&record_id],
            |row| run.row(row, |row| row.get::<_, i64>(0)),
        )?;
        let job_id = Uuid::new_v4().to_string();
        transaction.execute(run.sql(IndexSql::RepositoryQueueRetry3), [&record_id])?;
        transaction.execute(
            run.sql(IndexSql::RepositoryQueueRetry4),
            params![
                job_id,
                record_id,
                attempt,
                format!("embedding:{representation_id}:{attempt}")
            ],
        )?;
        let summary = latest_job(run, transaction, &record_id)?
            .ok_or(SemanticIndexError::InvalidInput("embedding job 创建失败"))?;
        Ok((summary, true))
    })
}

pub(crate) fn get_job_summary(
    run: &IndexRun<'_>,
    connection: &Connection,
    job_id: &str,
) -> Result<EmbeddingJobSummary, Error> {
    connection
        .query_row(
            run.sql(IndexSql::RepositoryGetJobSummary1),
            [job_id],
            |row| run.row(row, job_summary_from_row),
        )
        .optional()?
        .ok_or_else(|| SemanticIndexError::InvalidInput("找不到 embedding job").into())
}

pub(crate) fn get_status(
    run: &IndexRun<'_>,
    connection: &Connection,
) -> Result<SemanticIndexStatus, Error> {
    let mut statement = connection.prepare(run.sql(IndexSql::RepositoryGetStatus1))?;
    let spaces = statement
        .query_map([], |row| {
            run.row(row, |row| {
                Ok(SemanticIndexSpaceStatus {
                    embedding_space_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    model_id: row.get(2)?,
                    model_revision: row.get(3)?,
                    dimension: row.get(4)?,
                    generation: row.get(5)?,
                    generation_status: row.get(6)?,
                    namespace: row.get(7)?,
                    ready_count: row.get(8)?,
                    pending_count: row.get(9)?,
                    failed_count: row.get(10)?,
                    stale_count: row.get(11)?,
                })
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut jobs = connection.prepare(run.sql(IndexSql::RepositoryGetStatus2))?;
    let running_job_ids = jobs
        .query_map([], |row| run.row(row, |row| row.get::<_, String>(0)))?
        .collect::<Result<Vec<_>, _>>()?;
    let reconciliation_required = run.flag(IndexInputV1::Status {
        spaces: spaces.clone(),
    })?;
    Ok(SemanticIndexStatus {
        spaces,
        running_job_ids,
        reconciliation_required,
    })
}

pub(crate) fn mark_running(
    run: &IndexRun<'_>,
    connection: &mut Connection,
    job_id: &str,
    lease_owner: &str,
) -> Result<(), Error> {
    deferred(connection, |transaction| {
        let changed = transaction.execute(
            run.sql(IndexSql::RepositoryMarkRunning1),
            params![job_id, lease_owner],
        )?;
        run.validate(IndexInputV1::Running { changed })?;
        transaction.execute(run.sql(IndexSql::RepositoryMarkRunning2), [job_id])?;
        Ok(())
    })
}

pub(crate) fn mark_ready(
    run: &IndexRun<'_>,
    connection: &mut Connection,
    job: &PendingEmbeddingJob,
    dataset_version: u64,
    vector_norm: f32,
) -> Result<(), Error> {
    let dataset_version = i64::try_from(dataset_version)
        .map_err(|_| SemanticIndexError::InvalidInput("LanceDB dataset version 超出范围"))?;
    deferred(connection, |transaction| {
        let changed = transaction.execute(
            run.sql(IndexSql::RepositoryMarkReady1),
            params![job.record_id, dataset_version, f64::from(vector_norm)],
        )?;
        run.validate(IndexInputV1::Ready { changed })?;
        transaction.execute(
            run.sql(IndexSql::RepositoryMarkReady2),
            [job.job_id.as_str()],
        )?;
        transaction.execute(
            run.sql(IndexSql::RepositoryMarkReady3),
            params![job.record_id, dataset_version],
        )?;
        let generation = transaction.query_row(
            run.sql(IndexSql::RepositoryMarkReady4),
            [job.record_id.as_str()],
            |row| {
                run.row(row, |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                })
            },
        )?;
        if run.flag(IndexInputV1::Activate {
            status: generation.3,
            pending: generation.4,
        })? {
            transaction.execute(
                run.sql(IndexSql::RepositoryMarkReady5),
                params![generation.1, generation.2],
            )?;
            transaction.execute(run.sql(IndexSql::RepositoryMarkReady6), [generation.0])?;
        }
        Ok(())
    })
}

pub(crate) fn mark_interrupted(
    run: &IndexRun<'_>,
    connection: &Connection,
    job_id: &str,
) -> Result<(), Error> {
    connection.execute(run.sql(IndexSql::RepositoryMarkInterrupted1), [job_id])?;
    Ok(())
}

pub(crate) fn mark_failed(
    run: &IndexRun<'_>,
    connection: &Connection,
    job_id: &str,
    error_code: &str,
) -> Result<(), Error> {
    connection.execute(
        run.sql(IndexSql::RepositoryMarkFailed1),
        params![job_id, error_code],
    )?;
    connection.execute(
        run.sql(IndexSql::RepositoryMarkFailed2),
        params![job_id, error_code],
    )?;
    Ok(())
}

pub(crate) fn mark_canceled(
    run: &IndexRun<'_>,
    connection: &Connection,
    job_id: &str,
) -> Result<(), Error> {
    connection.execute(run.sql(IndexSql::RepositoryMarkCanceled1), [job_id])?;
    connection.execute(run.sql(IndexSql::RepositoryMarkCanceled2), [job_id])?;
    Ok(())
}

fn latest_job(
    run: &IndexRun<'_>,
    transaction: &Transaction<'_>,
    record_id: &str,
) -> Result<Option<EmbeddingJobSummary>, Error> {
    transaction
        .query_row(
            run.sql(IndexSql::RepositoryLatestJob1),
            [record_id],
            |row| run.row(row, job_summary_from_row),
        )
        .optional()
        .map_err(Into::into)
}

fn job_summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmbeddingJobSummary> {
    Ok(EmbeddingJobSummary {
        job_id: row.get(0)?,
        representation_id: row.get(1)?,
        subject_kind: row.get(2)?,
        subject_id: row.get(3)?,
        status: row.get(4)?,
        attempt: row.get(5)?,
        error_code: row.get(6)?,
        created_at: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

fn to_sql_error(error: SemanticIndexError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}
