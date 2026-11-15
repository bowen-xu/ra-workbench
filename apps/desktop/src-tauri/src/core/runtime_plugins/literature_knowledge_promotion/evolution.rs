//! Original evolution inputs and result writes. Native owns qualification and event kinds.
use super::{
    Decision, Input, KnowledgePolicy, KnowledgeSql, PromotionError, Result, algorithm_runs,
};
use crate::ai::{error::AiError, knowledge_contracts::KnowledgeError};
use research_workbench_native_sdk::knowledge_promotion::algorithms::{
    AlgorithmIdKindV1, ConceptEvolutionEvent,
};
use rusqlite::{Connection, Transaction, params};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn prepare_start<P: KnowledgePolicy>(
    connection: &Connection,
    sql: &KnowledgeSql,
    policy: &mut P,
    concept_id: &str,
) -> Result<algorithm_runs::PreparedRun, P::Pause> {
    let id = concept_id.trim();
    super::validation::checked_decision(policy.decide(
        0,
        Input::AlgorithmId {
            purpose: AlgorithmIdKindV1::Evolution,
            is_empty: id.is_empty(),
            bytes: id.len(),
        },
    )?)?;
    let exists = connection.query_row(
        sql.statement("concept-algorithm-evolution-exists.v1.sql"),
        [id],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        return Err(KnowledgeError::NotFound.into());
    }
    let paper_ids = evolution_paper_ids(connection, sql, id)?;
    let parameters = json!({"conceptId": id, "ordering": "issued_year_then_paper_id", "acceptedCurrentAssertionsOnly": true});
    algorithm_runs::prepare(connection, sql, &parameters, &paper_ids).map_err(Into::into)
}

pub(crate) fn prepare_events<P: KnowledgePolicy>(
    connection: &Connection,
    sql: &KnowledgeSql,
    policy: &mut P,
    concept_id: &str,
) -> Result<Vec<ConceptEvolutionEvent>, P::Pause> {
    let mut events = load_events(connection, sql, concept_id.trim())?;
    let snapshot_sha256 = serde_json::to_vec(&events)
        .map(|bytes| crate::runtime_plugins::files::digest(&bytes))
        .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?;
    let observations = events
        .iter()
        .map(|event| classification_token(&event.novelty_status))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut classifications = BTreeMap::new();
    for batch in 0..observations.len().div_ceil(64).max(1) {
        let end = ((batch + 1) * 64).min(observations.len());
        let chunk = &observations[batch * 64..end];
        let decision = policy.decide(
            batch + 1,
            Input::EvolutionEvents {
                snapshot_sha256,
                novelty_statuses: chunk.to_vec(),
            },
        )?;
        let Decision::EvolutionEvents { result: Ok(kinds) } = decision else {
            return Err(PromotionError::Failed(AiError::TemporaryUnavailable));
        };
        if kinds.len() != chunk.len() {
            return Err(PromotionError::Failed(AiError::TemporaryUnavailable));
        }
        classifications.extend(chunk.iter().cloned().zip(kinds));
    }
    for event in &mut events {
        let kind = classifications
            .get(&classification_token(&event.novelty_status))
            .ok_or(PromotionError::Failed(AiError::TemporaryUnavailable))?;
        event.event_kind = kind.as_str().to_owned();
    }
    Ok(events)
}

pub(crate) fn write_events(
    transaction: &Transaction<'_>,
    sql: &KnowledgeSql,
    run_id: &str,
    events: &[ConceptEvolutionEvent],
) -> std::result::Result<usize, KnowledgeError> {
    for (ordinal, event) in events.iter().enumerate() {
        transaction.execute(
            sql.statement("concept-algorithm-evolution-insert.v1.sql"),
            params![
                run_id,
                i64::try_from(ordinal).unwrap_or(i64::MAX),
                event.concept_id,
                event.concept_revision,
                event.concept_label,
                event.paper_id,
                event.paper_title,
                event.issued_year,
                event.assertion_id,
                event.assertion_revision,
                event.paper_function,
                event.novelty_status,
                event.event_kind,
                event.explanation,
                event.evidence_anchor_id,
            ],
        )?;
    }

    Ok(events.len())
}

fn evolution_paper_ids(
    connection: &Connection,
    sql: &KnowledgeSql,
    concept_id: &str,
) -> std::result::Result<Vec<String>, KnowledgeError> {
    let mut statement =
        connection.prepare(sql.statement("concept-algorithm-evolution-papers.v1.sql"))?;
    let rows = statement.query_map([concept_id], |row| row.get(0))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

fn load_events(
    connection: &Connection,
    sql: &KnowledgeSql,
    concept_id: &str,
) -> std::result::Result<Vec<ConceptEvolutionEvent>, KnowledgeError> {
    let mut statement =
        connection.prepare(sql.statement("concept-algorithm-evolution-source.v1.sql"))?;
    let rows = statement.query_map([concept_id], |row| {
        let novelty_status = row.get::<_, String>(9)?;
        Ok(ConceptEvolutionEvent {
            ordinal: 0,
            concept_id: row.get(0)?,
            concept_revision: row.get(1)?,
            concept_label: row.get(2)?,
            paper_id: row.get(3)?,
            paper_title: row.get(4)?,
            issued_year: row.get(5)?,
            assertion_id: row.get(6)?,
            assertion_revision: row.get(7)?,
            paper_function: row.get(8)?,
            event_kind: String::new(),
            novelty_status,
            explanation: row.get(10)?,
            evidence_anchor_id: row.get(11)?,
        })
    })?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

pub(crate) fn validate_run_id<P: KnowledgePolicy>(
    policy: &mut P,
    run_id: &str,
) -> Result<(), P::Pause> {
    super::validation::checked_decision(policy.decide(
        0,
        Input::AlgorithmId {
            purpose: AlgorithmIdKindV1::Run,
            is_empty: run_id.trim().is_empty(),
            bytes: run_id.len(),
        },
    )?)
}

pub(crate) fn insert_start(
    connection: &Connection,
    sql: &KnowledgeSql,
    prepared: &algorithm_runs::PreparedRun,
) -> std::result::Result<(), KnowledgeError> {
    algorithm_runs::insert(
        connection,
        sql,
        "concept_evolution",
        "accepted-paper-concept-timeline",
        1,
        prepared,
    )
}
#[cfg(test)]
pub(crate) fn start<P: KnowledgePolicy>(
    connection: &Connection,
    sql: &KnowledgeSql,
    policy: &mut P,
    concept_id: &str,
) -> Result<algorithm_runs::RunSnapshot, P::Pause> {
    let prepared = prepare_start(connection, sql, policy, concept_id)?;
    insert_start(connection, sql, &prepared)?;
    Ok(prepared.snapshot)
}

fn classification_token(status: &str) -> Option<String> {
    (status.len() <= 64).then(|| status.to_owned())
}
