//! The two original algorithm consumers share the existing Core stage and storage owners.
use super::{KnowledgePolicy, KnowledgeSql, Result, algorithm_runs, evolution, foundations};
use crate::ai::knowledge_contracts::KnowledgeError;
use research_workbench_native_sdk::knowledge_promotion::algorithms::{
    ConceptEvolutionEvent, PaperFoundationSimilarity, SharedFoundationsDraft,
};
use rusqlite::{Connection, Transaction};

#[derive(Clone)]
pub(crate) enum AlgorithmRequest {
    Evolution(String),
    Foundations(SharedFoundationsDraft),
}
pub(crate) enum AlgorithmRows {
    Evolution(Vec<ConceptEvolutionEvent>),
    Foundations(Vec<PaperFoundationSimilarity>),
}
impl AlgorithmRequest {
    pub(crate) fn source_message(&self) -> &'static str {
        match self {
            Self::Evolution(_) => "概念演化来源已变化，请重新分析。",
            Self::Foundations(_) => "共同基础分析来源已变化，请重新分析。",
        }
    }
    pub(crate) fn needs_foundation_resources(&self) -> bool {
        matches!(self, Self::Foundations(_))
    }
    pub(crate) fn prepare_start<P: KnowledgePolicy>(
        &self,
        db: &Connection,
        sql: &KnowledgeSql,
        policy: &mut P,
    ) -> Result<algorithm_runs::PreparedRun, P::Pause> {
        match self {
            Self::Evolution(id) => evolution::prepare_start(db, sql, policy, id),
            Self::Foundations(draft) => foundations::prepare_start(db, sql, policy, draft),
        }
    }
    pub(crate) fn insert_start(
        &self,
        db: &Connection,
        sql: &KnowledgeSql,
        prepared: &algorithm_runs::PreparedRun,
    ) -> std::result::Result<(), KnowledgeError> {
        match self {
            Self::Evolution(_) => evolution::insert_start(db, sql, prepared),
            Self::Foundations(_) => foundations::insert_start(db, sql, prepared),
        }
    }
    pub(crate) fn prepare_rows<P: KnowledgePolicy>(
        &self,
        db: &Connection,
        sql: &KnowledgeSql,
        policy: &mut P,
    ) -> Result<AlgorithmRows, P::Pause> {
        match self {
            Self::Evolution(id) => {
                evolution::prepare_events(db, sql, policy, id).map(AlgorithmRows::Evolution)
            }
            Self::Foundations(draft) => {
                foundations::prepare_results(db, sql, policy, draft).map(AlgorithmRows::Foundations)
            }
        }
    }
}
impl AlgorithmRows {
    pub(crate) fn write(
        &self,
        tx: &Transaction<'_>,
        sql: &KnowledgeSql,
        id: &str,
    ) -> std::result::Result<usize, KnowledgeError> {
        match self {
            Self::Evolution(events) => evolution::write_events(tx, sql, id, events),
            Self::Foundations(rows) => foundations::write_results(tx, sql, id, rows),
        }
    }
}
