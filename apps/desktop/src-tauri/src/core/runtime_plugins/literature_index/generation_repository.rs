use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::{decisions::IndexRun, failure::Error, model::SemanticIndexError, resources::IndexSql};
use research_workbench_native_sdk::semantic_index::{
    IndexGenerationSummary, SemanticNamespace, rules::IndexInputV1,
};

pub(crate) struct ActiveGeneration {
    pub id: String,
    pub embedding_space_id: String,
    pub generation: i64,
    pub namespace: SemanticNamespace,
}

pub(crate) fn ensure_current_generation(
    run: &IndexRun<'_>,
    connection: &Connection,
    namespace: SemanticNamespace,
) -> Result<Option<ActiveGeneration>, Error> {
    let settings = crate::ai::embedding_settings::configured_space_in(connection)?
        .map(|value| (value.region, value.model_id));
    let Some((region, model_id)) = settings else {
        return Ok(None);
    };
    let (space_id, region, model_id, dimension, instructions): (
        String,
        String,
        String,
        i64,
        String,
    ) = connection.query_row(
        run.sql(IndexSql::SemanticSpace),
        params![region, model_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    connection.execute(
        run.sql(IndexSql::GenerationRepositoryEnsureCurrentGeneration1),
        params![
            space_id,
            region.trim(),
            model_id.trim(),
            dimension,
            instructions
        ],
    )?;
    if let Some(generation) = active_generation(run, connection, &space_id, namespace)? {
        return Ok(Some(generation));
    }
    let generation = connection.query_row(
        run.sql(IndexSql::GenerationRepositoryEnsureCurrentGeneration2),
        params![space_id, namespace.table_name()],
        |row| row.get::<_, i64>(0),
    )?;
    let id = Uuid::new_v4().to_string();
    let relative_path: String = connection.query_row(
        run.sql(IndexSql::GenerationPath),
        params![space_id, generation],
        |row| row.get(0),
    )?;
    connection.execute(
        run.sql(IndexSql::GenerationRepositoryEnsureCurrentGeneration3),
        params![
            id,
            namespace.table_name(),
            space_id,
            generation,
            relative_path
        ],
    )?;
    Ok(Some(ActiveGeneration {
        id,
        embedding_space_id: space_id,
        generation,
        namespace,
    }))
}

pub(crate) fn queue_representation(
    run: &IndexRun<'_>,
    connection: &Connection,
    generation: &ActiveGeneration,
    representation_id: &str,
) -> Result<String, Error> {
    if let Some(job_id) = connection
        .query_row(
            run.sql(IndexSql::GenerationRepositoryQueueRepresentation1),
            params![
                representation_id,
                generation.embedding_space_id,
                generation.id
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(job_id);
    }
    let record_id = Uuid::new_v4().to_string();
    let job_id = Uuid::new_v4().to_string();
    let row_id: String = connection.query_row(
        run.sql(IndexSql::RowIdentity),
        params![
            representation_id,
            generation.embedding_space_id,
            generation.generation
        ],
        |row| row.get(0),
    )?;
    connection.execute(
        run.sql(IndexSql::GenerationRepositoryQueueRepresentation2),
        params![
            record_id,
            representation_id,
            generation.embedding_space_id,
            generation.id,
            generation.namespace.table_name(),
            row_id
        ],
    )?;
    connection.execute(
        run.sql(IndexSql::GenerationRepositoryQueueRepresentation3),
        params![job_id, record_id, format!("embedding:{record_id}:1")],
    )?;
    Ok(job_id)
}

pub(crate) fn queue_current_concept_backfill(
    run: &IndexRun<'_>,
    connection: &Connection,
) -> Result<i64, Error> {
    let Some(generation) =
        ensure_current_generation(run, connection, SemanticNamespace::AcceptedConcepts)?
    else {
        return Ok(0);
    };
    let representation_ids = connection
        .prepare(run.sql(IndexSql::GenerationRepositoryQueueCurrentConceptBackfill1))?
        .query_map([generation.id.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for representation_id in &representation_ids {
        queue_representation(run, connection, &generation, representation_id)?;
    }
    i64::try_from(representation_ids.len())
        .map_err(|_| SemanticIndexError::InvalidInput("backfill 数量超出范围").into())
}

pub(crate) fn create_rebuild_generation(
    run: &IndexRun<'_>,
    connection: &Connection,
    embedding_space_id: &str,
) -> Result<IndexGenerationSummary, Error> {
    let namespace = SemanticNamespace::AcceptedConcepts;
    let space_exists = connection.query_row(
        run.sql(IndexSql::GenerationRepositoryCreateRebuildGeneration1),
        [embedding_space_id],
        |row| row.get::<_, bool>(0),
    )?;
    run.validate(IndexInputV1::Space {
        exists: space_exists,
    })?;
    if let Some(id) = connection
        .query_row(
            run.sql(IndexSql::GenerationRepositoryCreateRebuildGeneration2),
            params![embedding_space_id, namespace.table_name()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return generation_summary(run, connection, &id);
    }
    let generation = connection.query_row(
        run.sql(IndexSql::GenerationRepositoryCreateRebuildGeneration3),
        params![embedding_space_id, namespace.table_name()],
        |row| row.get::<_, i64>(0),
    )?;
    let id = Uuid::new_v4().to_string();
    let relative_path: String = connection.query_row(
        run.sql(IndexSql::GenerationPath),
        params![embedding_space_id, generation],
        |row| row.get(0),
    )?;
    connection.execute(
        run.sql(IndexSql::GenerationRepositoryCreateRebuildGeneration4),
        params![
            id,
            namespace.table_name(),
            embedding_space_id,
            generation,
            relative_path
        ],
    )?;
    let generation_ref = ActiveGeneration {
        id: id.clone(),
        embedding_space_id: embedding_space_id.to_owned(),
        generation,
        namespace,
    };
    let representation_ids = latest_concept_representations(run, connection)?;
    for representation_id in &representation_ids {
        queue_representation(run, connection, &generation_ref, representation_id)?;
    }
    if representation_ids.is_empty() {
        activate_empty_generation(run, connection, &generation_ref)?;
    }
    generation_summary(run, connection, &id)
}

fn latest_concept_representations(
    run: &IndexRun<'_>,
    connection: &Connection,
) -> Result<Vec<String>, Error> {
    connection
        .prepare(run.sql(IndexSql::GenerationRepositoryLatestConceptRepresentations1))?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn activate_empty_generation(
    run: &IndexRun<'_>,
    connection: &Connection,
    generation: &ActiveGeneration,
) -> Result<(), Error> {
    connection.execute(
        run.sql(IndexSql::GenerationRepositoryActivateEmptyGeneration1),
        params![
            generation.namespace.table_name(),
            generation.embedding_space_id
        ],
    )?;
    connection.execute(
        run.sql(IndexSql::GenerationRepositoryActivateEmptyGeneration2),
        [&generation.id],
    )?;
    Ok(())
}

fn generation_summary(
    run: &IndexRun<'_>,
    connection: &Connection,
    generation_id: &str,
) -> Result<IndexGenerationSummary, Error> {
    connection
        .query_row(
            run.sql(IndexSql::GenerationRepositoryGenerationSummary1),
            [generation_id],
            |row| {
                Ok(IndexGenerationSummary {
                    generation_id: row.get(0)?,
                    embedding_space_id: row.get(1)?,
                    namespace: row.get(2)?,
                    generation: row.get(3)?,
                    status: row.get(4)?,
                    record_count: row.get(5)?,
                    ready_count: row.get(6)?,
                    created_at: row.get(7)?,
                    activated_at: row.get(8)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| SemanticIndexError::InvalidInput("找不到 index generation").into())
}

fn active_generation(
    run: &IndexRun<'_>,
    connection: &Connection,
    space_id: &str,
    namespace: SemanticNamespace,
) -> Result<Option<ActiveGeneration>, Error> {
    connection
        .query_row(
            run.sql(IndexSql::GenerationRepositoryActiveGeneration1),
            params![space_id, namespace.table_name()],
            |row| {
                Ok(ActiveGeneration {
                    id: row.get(0)?,
                    embedding_space_id: space_id.to_owned(),
                    generation: row.get(1)?,
                    namespace,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}
