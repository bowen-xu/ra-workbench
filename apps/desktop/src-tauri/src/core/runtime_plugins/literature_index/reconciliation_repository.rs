use crate::semantic_engine::SemanticNamespace;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::{decisions::IndexRun, failure::Error, model::SemanticIndexError, resources::IndexSql};

pub(crate) use research_workbench_native_sdk::semantic_index::{
    ReconciliationCounts, ReconciliationGeneration, ReconciliationRecord,
};

pub(crate) fn begin_run(run: &IndexRun<'_>, connection: &Connection) -> Result<String, Error> {
    let run_id = Uuid::new_v4().to_string();
    connection.execute(
        run.sql(IndexSql::ReconciliationRepositoryBeginRun1),
        [&run_id],
    )?;
    Ok(run_id)
}

pub(crate) fn load_generations(
    run: &IndexRun<'_>,
    connection: &Connection,
) -> Result<Vec<ReconciliationGeneration>, Error> {
    let mut statement =
        connection.prepare(run.sql(IndexSql::ReconciliationRepositoryLoadGenerations1))?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut generations = Vec::new();
    for row in rows {
        let (id, namespace, relative_database_path, dimension) = row?;
        generations.push(ReconciliationGeneration {
            records: load_records(run, connection, &id)?,
            namespace: parse_namespace(&namespace)?,
            relative_database_path,
            dimension: usize::try_from(dimension)
                .map_err(|_| SemanticIndexError::InvalidInput("向量维度无效"))?,
        });
    }
    Ok(generations)
}

pub(crate) fn mark_stale(
    run: &IndexRun<'_>,
    connection: &mut Connection,
    record: &ReconciliationRecord,
) -> Result<(), Error> {
    connection.execute(
        run.sql(IndexSql::ReconciliationRepositoryMarkStaleAndQueue1),
        [&record.record_id],
    )?;
    Ok(())
}

pub(crate) fn finish_run(
    run: &IndexRun<'_>,
    connection: &Connection,
    run_id: &str,
    status: &str,
    counts: ReconciliationCounts,
    error_code: Option<&str>,
) -> Result<(), Error> {
    connection.execute(
        run.sql(IndexSql::ReconciliationRepositoryFinishRun1),
        params![
            run_id,
            status,
            counts.checked,
            counts.missing,
            counts.orphan,
            counts.repaired,
            error_code
        ],
    )?;
    Ok(())
}

fn load_records(
    run: &IndexRun<'_>,
    connection: &Connection,
    generation_id: &str,
) -> Result<Vec<ReconciliationRecord>, Error> {
    let mut statement =
        connection.prepare(run.sql(IndexSql::ReconciliationRepositoryLoadRecords1))?;
    statement
        .query_map([generation_id], |row| {
            Ok(ReconciliationRecord {
                record_id: row.get(0)?,
                representation_id: row.get(1)?,
                lance_row_id: row.get(2)?,
                input_sha256: row.get(3)?,
                status: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn parse_namespace(value: &str) -> Result<SemanticNamespace, Error> {
    research_workbench_native_sdk::semantic_index::decode_namespace(value)
        .map_err(|message| SemanticIndexError::InvalidInput(message).into())
}

pub(crate) fn get_summary(
    run: &IndexRun<'_>,
    connection: &Connection,
    run_id: &str,
) -> Result<research_workbench_native_sdk::semantic_index::ReconciliationSummary, Error> {
    connection
        .query_row(
            run.sql(IndexSql::ReconciliationRepositoryGetSummary1),
            [run_id],
            |row| {
                Ok(
                    research_workbench_native_sdk::semantic_index::ReconciliationSummary {
                        run_id: row.get(0)?,
                        status: row.get(1)?,
                        checked_count: row.get(2)?,
                        missing_count: row.get(3)?,
                        orphan_count: row.get(4)?,
                        repaired_count: row.get(5)?,
                        error_code: row.get(6)?,
                        started_at: row.get(7)?,
                        finished_at: row.get(8)?,
                    },
                )
            },
        )
        .optional()?
        .ok_or_else(|| SemanticIndexError::InvalidInput("找不到 reconciliation run").into())
}
