//! Original accepted-knowledge writes; Core retains the actual review transaction.
mod acceptance;
pub(crate) mod algorithm_io;
pub(crate) mod algorithm_runs;
pub(crate) mod capture;
mod concept;
mod evidence;
pub(crate) mod evolution;
pub(crate) mod foundations;
pub(crate) mod identity;
pub(crate) mod manual;
mod paper;
mod read;
pub(crate) mod relation;
use read::KnowledgeRead;
mod resources;
mod validation;
use crate::ai::{error::AiError, knowledge_contracts::KnowledgeError};
use research_workbench_native_sdk::knowledge_promotion::{
    KnowledgeValidationDecisionV1 as Decision, KnowledgeValidationInputV1 as Input,
};
use research_workbench_native_sdk::{
    concepts::{Concept, NewConceptDetails, Term},
    knowledge_promotion::{models::*, *},
};
pub(crate) use resources::KnowledgeSql;
use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;
#[derive(Debug)]
pub(crate) enum PromotionError<P> {
    Failed(AiError),
    Need(P),
}
impl<P> From<KnowledgeError> for PromotionError<P> {
    fn from(error: KnowledgeError) -> Self {
        Self::Failed(AiError::Knowledge(error))
    }
}
impl<P> From<rusqlite::Error> for PromotionError<P> {
    fn from(error: rusqlite::Error) -> Self {
        KnowledgeError::Database(error).into()
    }
}
type Result<T, P> = std::result::Result<T, PromotionError<P>>;
pub(crate) struct AiAcceptedKnowledgeTarget {
    pub(crate) kind: &'static str,
    pub(crate) id: String,
    pub(crate) revision: i64,
}
pub(crate) struct DefinitionClaimOrigin<'a> {
    pub(crate) provenance_kind: &'a str,
    pub(crate) editor_kind: &'a str,
    pub(crate) reason: &'a str,
}
pub(crate) trait KnowledgePolicy {
    type Pause;
    fn decide(&mut self, ordinal: usize, input: Input) -> Result<Decision, Self::Pause>;
    fn register_semantic(
        &mut self,
        tx: &Transaction<'_>,
        concept: &Concept,
    ) -> Result<(), Self::Pause>;
}
pub(crate) struct KnowledgeIo<'a, 'db, P: KnowledgePolicy> {
    db: &'a Transaction<'db>,
    sql: &'a KnowledgeSql,
    policy: &'a mut P,
    ordinal: usize,
}
impl<'a, 'db, P: KnowledgePolicy> KnowledgeIo<'a, 'db, P> {
    pub(crate) fn new(db: &'a Transaction<'db>, sql: &'a KnowledgeSql, policy: &'a mut P) -> Self {
        Self {
            db,
            sql,
            policy,
            ordinal: 0,
        }
    }
    pub(crate) fn promote(
        &mut self,
        plan: KnowledgePromotionV1,
        provenance: &str,
    ) -> Result<AiAcceptedKnowledgeTarget, P::Pause> {
        match plan {
            KnowledgePromotionV1::PaperConcept(draft) => {
                self.promote_ai_paper_concept(draft, provenance)
            }
            KnowledgePromotionV1::Concept(draft) => self.promote_ai_concept(draft, provenance),
            KnowledgePromotionV1::Definition(draft) => {
                self.promote_ai_definition(draft, provenance)
            }
            KnowledgePromotionV1::Relation(draft) => self.promote_ai_relation(draft, provenance),
        }
    }
}

pub(crate) fn validation_rejection(decision: &Decision) -> Option<String> {
    match decision {
        Decision::FoundationComparison { .. }
        | Decision::Capture { .. }
        | Decision::DefinitionRevision { .. }
        | Decision::RelationRevision { .. } => None,
        Decision::RelationNode { result } => result.as_ref().err().cloned(),
        Decision::RelationSearch { result } => result.as_ref().err().cloned(),
        Decision::EvolutionEvents { result } => result.as_ref().err().cloned(),
        Decision::Checked { rejection } => rejection.clone(),
        Decision::PaperConcept { result } => result.as_ref().err().cloned(),
        Decision::Term { prepared } => prepared.rejection.map(|reason| reason.message().into()),
        Decision::Details { result } => result.as_ref().err().cloned(),
        Decision::Definition { result } => result.as_ref().err().cloned(),
    }
}
