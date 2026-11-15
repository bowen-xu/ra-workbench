use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::KnowledgeSql;
use crate::ai::knowledge_contracts::KnowledgeError;
use research_workbench_native_sdk::knowledge_promotion::algorithms::{
    ConceptAlgorithmRun, ConceptEvolutionEvent, PaperFoundationSimilarity, SharedFoundationConcept,
};

pub(crate) struct RunSnapshot {
    pub id: String,
    pub parameters_hash: String,
    pub paper_scope_hash: String,
    pub concept_revision_watermark: i64,
    pub relation_revision_watermark: i64,
}

#[cfg(test)]
pub(crate) fn start(
    connection: &Connection,
    sql: &KnowledgeSql,
    algorithm_kind: &str,
    algorithm_id: &str,
    algorithm_version: i64,
    parameters: &Value,
    paper_ids: &[String],
) -> Result<RunSnapshot, KnowledgeError> {
    let prepared = prepare(connection, sql, parameters, paper_ids)?;
    insert(
        connection,
        sql,
        algorithm_kind,
        algorithm_id,
        algorithm_version,
        &prepared,
    )?;
    Ok(prepared.snapshot)
}

pub(crate) struct PreparedRun {
    pub(crate) snapshot: RunSnapshot,
    parameters_json: String,
    paper_scope_json: String,
}
pub(crate) fn prepare(
    connection: &Connection,
    sql: &KnowledgeSql,
    parameters: &Value,
    paper_ids: &[String],
) -> Result<PreparedRun, KnowledgeError> {
    let parameters_json = serde_json::to_string(parameters)
        .map_err(|_| KnowledgeError::InvalidInput("算法参数无法序列化".to_owned()))?;
    let paper_scope_json = serde_json::to_string(paper_ids)
        .map_err(|_| KnowledgeError::InvalidInput("论文范围无法序列化".to_owned()))?;
    let snapshot = RunSnapshot {
        id: Uuid::new_v4().to_string(),
        parameters_hash: hash(&parameters_json),
        paper_scope_hash: hash(&paper_scope_json),
        concept_revision_watermark: maximum_revision(connection, sql, "concepts")?,
        relation_revision_watermark: maximum_revision(connection, sql, "relation_assertions")?,
    };

    Ok(PreparedRun {
        snapshot,
        parameters_json,
        paper_scope_json,
    })
}
pub(crate) fn insert(
    connection: &Connection,
    sql: &KnowledgeSql,
    algorithm_kind: &str,
    algorithm_id: &str,
    algorithm_version: i64,
    prepared: &PreparedRun,
) -> Result<(), KnowledgeError> {
    let snapshot = &prepared.snapshot;
    connection.execute(
        sql.statement("concept-algorithm-run-insert.v1.sql"),
        params![
            snapshot.id,
            algorithm_kind,
            algorithm_id,
            algorithm_version,
            prepared.parameters_json,
            snapshot.parameters_hash,
            snapshot.concept_revision_watermark,
            snapshot.relation_revision_watermark,
            prepared.paper_scope_json,
            snapshot.paper_scope_hash,
        ],
    )?;

    Ok(())
}

pub(crate) fn finish(
    connection: &Connection,
    sql: &KnowledgeSql,
    run_id: &str,
    result_count: usize,
) -> Result<(), KnowledgeError> {
    connection.execute(
        sql.statement("concept-algorithm-run-finish.v1.sql"),
        params![run_id, i64::try_from(result_count).unwrap_or(i64::MAX)],
    )?;
    Ok(())
}

pub(crate) fn fail(connection: &Connection, sql: &KnowledgeSql, run_id: &str) {
    let _ = connection.execute(sql.statement("concept-algorithm-run-fail.v1.sql"), [run_id]);
}

pub(crate) fn get(
    connection: &Connection,
    sql: &KnowledgeSql,
    run_id: &str,
) -> Result<ConceptAlgorithmRun, KnowledgeError> {
    let mut run = connection
        .query_row(
            sql.statement("concept-algorithm-run-read.v1.sql"),
            [run_id],
            |row| {
                Ok(ConceptAlgorithmRun {
                    id: row.get(0)?,
                    algorithm_kind: row.get(1)?,
                    algorithm_id: row.get(2)?,
                    algorithm_version: row.get(3)?,
                    parameters_hash: row.get(4)?,
                    concept_revision_watermark: row.get(5)?,
                    relation_revision_watermark: row.get(6)?,
                    paper_scope_hash: row.get(7)?,
                    status: row.get(8)?,
                    result_count: row.get(9)?,
                    created_at: row.get(10)?,
                    started_at: row.get(11)?,
                    finished_at: row.get(12)?,
                    evolution_events: Vec::new(),
                    foundation_similarities: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or(KnowledgeError::NotFound)?;
    run.evolution_events = load_evolution(connection, sql, run_id)?;
    run.foundation_similarities = load_similarities(connection, sql, run_id)?;
    Ok(run)
}

fn load_evolution(
    connection: &Connection,
    sql: &KnowledgeSql,
    run_id: &str,
) -> Result<Vec<ConceptEvolutionEvent>, KnowledgeError> {
    let mut statement =
        connection.prepare(sql.statement("concept-algorithm-run-evolution.v1.sql"))?;
    let rows = statement.query_map([run_id], |row| {
        Ok(ConceptEvolutionEvent {
            ordinal: row.get(0)?,
            concept_id: row.get(1)?,
            concept_revision: row.get(2)?,
            concept_label: row.get(3)?,
            paper_id: row.get(4)?,
            paper_title: row.get(5)?,
            issued_year: row.get(6)?,
            assertion_id: row.get(7)?,
            assertion_revision: row.get(8)?,
            paper_function: row.get(9)?,
            novelty_status: row.get(10)?,
            event_kind: row.get(11)?,
            explanation: row.get(12)?,
            evidence_anchor_id: row.get(13)?,
        })
    })?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}

fn load_similarities(
    connection: &Connection,
    sql: &KnowledgeSql,
    run_id: &str,
) -> Result<Vec<PaperFoundationSimilarity>, KnowledgeError> {
    let mut statement =
        connection.prepare(sql.statement("concept-algorithm-run-foundations.v1.sql"))?;
    let rows = statement.query_map([run_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, f64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, i64>(9)?,
        ))
    })?;
    rows.map(|row| {
        let row = row?;
        let shared_concepts = serde_json::from_str::<Vec<SharedFoundationConcept>>(&row.5)
            .map_err(|_| KnowledgeError::InvalidInput("共享基础概念结果已损坏".to_owned()))?;
        Ok(PaperFoundationSimilarity {
            ordinal: row.0,
            left_paper_id: row.1,
            left_paper_title: row.2,
            right_paper_id: row.3,
            right_paper_title: row.4,
            shared_concepts,
            weighted_jaccard: row.6,
            shared_concept_count: row.7,
            left_foundation_count: row.8,
            right_foundation_count: row.9,
        })
    })
    .collect()
}

fn maximum_revision(
    connection: &Connection,
    sql: &KnowledgeSql,
    table: &str,
) -> Result<i64, KnowledgeError> {
    let statement = match table {
        "concepts" => sql.statement("concept-algorithm-run-concept-watermark.v1.sql"),
        "relation_assertions" => sql.statement("concept-algorithm-run-relation-watermark.v1.sql"),
        _ => return Err(KnowledgeError::InvalidInput("算法版本水位无效".to_owned())),
    };
    connection
        .query_row(statement, [], |row| row.get(0))
        .map_err(Into::into)
}

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
