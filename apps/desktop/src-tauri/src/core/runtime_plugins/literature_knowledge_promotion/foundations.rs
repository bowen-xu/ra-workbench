//! Original foundation rows and writes. Native owns qualification and the complete pure algorithm.
use super::{
    Decision, Input, KnowledgePolicy, KnowledgeSql, PromotionError, Result, algorithm_runs,
};
use crate::ai::{error::AiError, knowledge_contracts::KnowledgeError};
use research_workbench_native_sdk::{
    knowledge_promotion::{
        algorithms::{PaperFoundationSimilarity, SharedFoundationsDraft},
        foundations::*,
    },
    paper_context_wire::decode_payload,
};
use rusqlite::{Connection, Transaction, params};
use serde_json::json;
use std::collections::BTreeMap;

fn normalized_ids(draft: &SharedFoundationsDraft) -> Vec<String> {
    let mut ids = draft
        .paper_ids
        .iter()
        .map(|id| id.trim().to_owned())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}
pub(crate) fn prepare_start<P: KnowledgePolicy>(
    db: &Connection,
    sql: &KnowledgeSql,
    policy: &mut P,
    draft: &SharedFoundationsDraft,
) -> Result<algorithm_runs::PreparedRun, P::Pause> {
    let ids = normalized_ids(draft);
    super::validation::checked_decision(
        policy.decide(
            0,
            Input::FoundationScope {
                invalid_id: draft
                    .paper_ids
                    .iter()
                    .any(|id| id.trim().is_empty() || id.trim().len() > 160),
                unique_papers: ids.len(),
            },
        )?,
    )?;
    let ids_json = serde_json::to_string(&ids)
        .map_err(|_| KnowledgeError::InvalidInput("论文范围无法序列化".into()))?;
    let count = db.query_row(
        sql.statement("concept-foundations-scope.v1.sql"),
        [ids_json],
        |row| row.get::<_, usize>(0),
    )?;
    if count != ids.len() {
        return Err(KnowledgeError::NotFound.into());
    }
    let parameters = json!({"weighting":"inverse_log_document_frequency","acceptedCurrentAssertionsOnly":true,"paperCount":ids.len()});
    algorithm_runs::prepare(db, sql, &parameters, &ids).map_err(Into::into)
}
pub(crate) fn insert_start(
    db: &Connection,
    sql: &KnowledgeSql,
    prepared: &algorithm_runs::PreparedRun,
) -> std::result::Result<(), KnowledgeError> {
    algorithm_runs::insert(
        db,
        sql,
        "shared_foundations",
        "foundational-concept-weighted-jaccard",
        1,
        prepared,
    )
}
pub(crate) fn prepare_results<P: KnowledgePolicy>(
    db: &Connection,
    sql: &KnowledgeSql,
    policy: &mut P,
    draft: &SharedFoundationsDraft,
) -> Result<Vec<PaperFoundationSimilarity>, P::Pause> {
    let paper_ids = normalized_ids(draft);
    let papers = load_foundations(db, sql, &paper_ids)?;
    let decision = policy.decide(
        1,
        Input::FoundationComparison {
            input: Box::new(FoundationInputV1 { paper_ids, papers }),
        },
    )?;
    let Decision::FoundationComparison {
        result: Ok(encoded),
    } = decision
    else {
        return Err(PromotionError::Failed(AiError::TemporaryUnavailable));
    };
    let decoded: std::result::Result<Vec<PaperFoundationSimilarity>, FoundationRejectionV1> =
        decode_payload(&encoded)
            .map_err(|_| PromotionError::Failed(AiError::TemporaryUnavailable))?;
    decoded.map_err(|error| match error {
        FoundationRejectionV1::NotFound => KnowledgeError::NotFound.into(),
        FoundationRejectionV1::InvalidInput { message } => {
            KnowledgeError::InvalidInput(message).into()
        }
    })
}
pub(crate) fn write_results(
    transaction: &Transaction<'_>,
    sql: &KnowledgeSql,
    run_id: &str,
    results: &[PaperFoundationSimilarity],
) -> std::result::Result<usize, KnowledgeError> {
    for (ordinal, result) in results.iter().enumerate() {
        let shared_json = serde_json::to_string(&result.shared_concepts)
            .map_err(|_| KnowledgeError::InvalidInput("共享基础概念无法序列化".to_owned()))?;
        transaction.execute(
            sql.statement("concept-foundations-insert.v1.sql"),
            params![
                run_id,
                i64::try_from(ordinal).unwrap_or(i64::MAX),
                result.left_paper_id,
                result.left_paper_title,
                result.right_paper_id,
                result.right_paper_title,
                shared_json,
                result.weighted_jaccard,
                result.shared_concept_count,
                result.left_foundation_count,
                result.right_foundation_count,
            ],
        )?;
    }

    Ok(results.len())
}
fn load_foundations(
    connection: &Connection,
    sql: &KnowledgeSql,
    paper_ids: &[String],
) -> std::result::Result<BTreeMap<String, PaperFoundations>, KnowledgeError> {
    let ids_json = serde_json::to_string(paper_ids)
        .map_err(|_| KnowledgeError::InvalidInput("论文范围无法序列化".to_owned()))?;
    let mut papers = BTreeMap::new();
    let mut paper_statement =
        connection.prepare(sql.statement("concept-foundations-papers.v1.sql"))?;
    for row in paper_statement.query_map([&ids_json], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (id, title) = row?;
        papers.insert(
            id,
            PaperFoundations {
                title,
                concepts: BTreeMap::new(),
            },
        );
    }
    let mut statement = connection.prepare(sql.statement("concept-foundations-sources.v1.sql"))?;
    for row in statement.query_map([ids_json], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            FoundationIdentity {
                revision: row.get(2)?,
                label: row.get(3)?,
            },
        ))
    })? {
        let (paper_id, concept_id, identity) = row?;
        if let Some(paper) = papers.get_mut(&paper_id) {
            paper.concepts.insert(concept_id, identity);
        }
    }
    Ok(papers)
}
