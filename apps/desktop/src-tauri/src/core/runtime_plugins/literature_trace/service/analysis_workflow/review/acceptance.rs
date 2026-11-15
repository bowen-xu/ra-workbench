//! Recheck the original review prefix after guard-free finite Native decisions.
use super::super::*;
use crate::{
    ai::{
        candidate_contracts::CandidatePayload,
        error::AiError,
        review_support::{
            AiCandidate, AiReviewDecisionDraft, GenericReviewAdapter, ReviewPlan, ReviewPromotion,
            decide_with,
        },
        structured_resource_validation::Flow,
    },
    data_interface::guarded_transaction,
    runtime_plugins::{
        data,
        literature_reference_records::acceptance::{
            ReferenceAcceptanceError, ReferenceAcceptanceSql,
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::{
    analysis_workflow::candidate::{CandidateDecisionV1, CandidateInputV1},
    candidate_review::{
        reference::{ReferencePromotionInputV1, ReferencePromotionSourceV1},
        wire::{ReviewDecisionV1, ReviewInputV1},
    },
};
use rusqlite::{Connection, Transaction, params};
mod derived;
mod generic;
mod items;
mod paper;

#[derive(Clone)]
enum ReviewRequest {
    Candidate(AiReviewDecisionDraft),
    Paper(research_workbench_native_sdk::candidate_review::items::AiSuggestionItemDecisionDraft),
    Suggestion(
        research_workbench_native_sdk::candidate_review::items::AiSuggestionItemDecisionDraft,
    ),
}
impl ReviewRequest {
    fn candidate_id(&self) -> &str {
        match self {
            Self::Candidate(draft) => &draft.candidate_id,
            Self::Suggestion(draft) | Self::Paper(draft) => &draft.candidate_id,
        }
    }
}
struct Observation {
    input: WorkflowInputV1,
    decision: WorkflowDecisionV1,
    origin: DecisionOrigin,
}
#[derive(Default)]
struct ReviewPolicy {
    observations: Vec<Observation>,
    knowledge: Option<crate::runtime_plugins::literature_knowledge_promotion::KnowledgeSql>,
    semantic: Option<crate::runtime_plugins::literature_concepts::ConceptResources>,
    references: Option<ReferenceAcceptanceSql>,
    project_sql: Option<String>,
    derived: Option<crate::runtime_plugins::literature_derived_view::DerivedAcceptanceSql>,
    reading_units: Option<
        crate::runtime_plugins::literature_reading_units::accepted::ReadingUnitAcceptanceSql,
    >,
}
enum Need {
    Decision(Box<WorkflowInputV1>),
    References,
    Project,
    Derived,
    ReadingUnits,
    Knowledge,
    PaperKnowledge,
}
impl ReviewPolicy {
    fn observed(&self, input: WorkflowInputV1) -> Result<WorkflowDecisionV1, Flow<AiError, Need>> {
        // Replay may become authoritative while another Native decision is pending.
        // Match rule identity, then compare that rule's exact owned input.
        let previous = self
            .observations
            .iter()
            .find(|entry| same_rule(&entry.input, &input));
        let Some(previous) = previous else {
            return Err(Flow::Need(Need::Decision(Box::new(input))));
        };
        if previous.input != input {
            return Err(Flow::original(template::identified(
                "plugin_host_source_changed",
                "候选审阅来源已变化，请重新载入。",
                &previous.origin,
            )));
        }
        Ok(previous.decision.clone())
    }
}
fn same_rule(left: &WorkflowInputV1, right: &WorkflowInputV1) -> bool {
    match (left, right) {
        (
            WorkflowInputV1::PaperConceptReview { input: a },
            WorkflowInputV1::PaperConceptReview { input: b },
        ) => std::mem::discriminant(a.as_ref()) == std::mem::discriminant(b.as_ref()),
        (
            WorkflowInputV1::SuggestionReview { input: a },
            WorkflowInputV1::SuggestionReview { input: b },
        ) => {
            use research_workbench_native_sdk::candidate_review::items::ItemReviewInputV1 as Input;
            match (a.as_ref(), b.as_ref()) {
                (Input::Knowledge { ordinal: a, .. }, Input::Knowledge { ordinal: b, .. }) => {
                    a == b
                }
                _ => std::mem::discriminant(a.as_ref()) == std::mem::discriminant(b.as_ref()),
            }
        }
        (
            WorkflowInputV1::CandidateReview { input: a },
            WorkflowInputV1::CandidateReview { input: b },
        ) if matches!(
            (a.as_ref(), b.as_ref()),
            (
                ReviewInputV1::ReadingUnitValidation { .. },
                ReviewInputV1::ReadingUnitValidation { .. }
            )
        ) =>
        {
            let (
                ReviewInputV1::ReadingUnitValidation { ordinal: a, .. },
                ReviewInputV1::ReadingUnitValidation { ordinal: b, .. },
            ) = (a.as_ref(), b.as_ref())
            else {
                unreachable!("matched reading validation");
            };
            a == b
        }
        (
            WorkflowInputV1::CandidateReview { input: a },
            WorkflowInputV1::CandidateReview { input: b },
        ) if matches!(
            (a.as_ref(), b.as_ref()),
            (
                ReviewInputV1::DerivedValidation { .. },
                ReviewInputV1::DerivedValidation { .. }
            )
        ) =>
        {
            let (
                ReviewInputV1::DerivedValidation { input: a },
                ReviewInputV1::DerivedValidation { input: b },
            ) = (a.as_ref(), b.as_ref())
            else {
                unreachable!("matched validation");
            };
            a.same_rule(b)
        }
        (
            WorkflowInputV1::CandidateReview { input: a },
            WorkflowInputV1::CandidateReview { input: b },
        ) => std::mem::discriminant(a.as_ref()) == std::mem::discriminant(b.as_ref()),
        (WorkflowInputV1::Candidate { .. }, WorkflowInputV1::Candidate { .. }) => true,
        _ => false,
    }
}
impl GenericReviewAdapter for ReviewPolicy {
    type Pause = Need;
    fn review(&mut self, input: ReviewInputV1) -> Result<ReviewDecisionV1, Flow<AiError, Need>> {
        let request = WorkflowInputV1::CandidateReview {
            input: Box::new(input),
        };
        let decision = self.observed(request.clone())?;
        let origin = &self
            .observations
            .iter()
            .find(|entry| same_rule(&entry.input, &request))
            .expect("observed rule")
            .origin;
        let WorkflowDecisionV1::CandidateReview { decision } = decision else {
            return Err(Flow::original(template::identified(
                "plugin_host_response_mismatch",
                "候选审阅结果种类不匹配。",
                origin,
            )));
        };
        let error = match &decision {
            ReviewDecisionV1::GenericPromotion { result } => result.as_ref().err(),
            ReviewDecisionV1::ReadingUnitValidation { rejection } => rejection.as_ref(),
            ReviewDecisionV1::Validated { rejection } => rejection.as_ref(),
            ReviewDecisionV1::Candidate { result } => result.as_ref().err(),
            ReviewDecisionV1::ReferencePromotion { result } => result.as_ref().err(),
            ReviewDecisionV1::DerivedPromotion { result } => result.as_ref().err(),
            ReviewDecisionV1::DerivedValidation { rejection } => rejection.as_ref(),
        };
        if let Some(error) = error {
            return Err(Flow::original(template::rejected(error.clone(), origin)));
        }
        Ok(decision)
    }
    fn decode(
        &mut self,
        candidate: &AiCandidate,
        value: serde_json::Value,
    ) -> Result<CandidatePayload, Flow<AiError, Need>> {
        let input = WorkflowInputV1::Candidate {
            input: Box::new(CandidateInputV1::Decode {
                content: value.to_string(),
                evidence_keys: candidate
                    .evidence_bundle
                    .items
                    .iter()
                    .map(|item| item.citation_key.clone())
                    .collect(),
            }),
        };
        let decision = self.observed(input.clone())?;
        let origin = &self
            .observations
            .iter()
            .find(|entry| same_rule(&entry.input, &input))
            .expect("observed decoder")
            .origin;
        match decision {
            WorkflowDecisionV1::Candidate {
                decision:
                    CandidateDecisionV1::Decoded {
                        payload_cbor_base64,
                    },
            } => decode_payload(&payload_cbor_base64).map_err(|_| {
                Flow::original(template::identified(
                    "plugin_host_response_mismatch",
                    "候选审阅解码结果格式不匹配。",
                    origin,
                ))
            }),
            WorkflowDecisionV1::Candidate {
                decision: CandidateDecisionV1::Rejected { error },
            } => Err(Flow::original(template::rejected(error, origin))),
            _ => Err(Flow::original(template::identified(
                "plugin_host_response_mismatch",
                "候选审阅解码结果种类不匹配。",
                origin,
            ))),
        }
    }
    fn project_contains_paper(
        &mut self,
        db: &Connection,
        project: &str,
        paper: &str,
    ) -> Result<bool, Flow<AiError, Need>> {
        let Some(sql) = &self.project_sql else {
            return Err(Flow::Need(Need::Project));
        };
        db.query_row(sql.trim_end_matches('\n'), params![project, paper], |row| {
            row.get(0)
        })
        .map_err(|error| Flow::original(error.into()))
    }
    fn promote(&mut self, tx: &Transaction<'_>, plan: &ReviewPlan<'_>) -> ReviewPromotion<Need> {
        if let CandidatePayload::DerivedResearchView(view) = plan.payload {
            return self.promote_derived(tx, plan, view);
        }
        let CandidatePayload::ReferenceRecord(reference) = plan.payload else {
            return self.promote_generic(tx, plan);
        };
        let input = ReferencePromotionInputV1 {
            reference: reference.clone(),
            sources: plan
                .candidate
                .evidence_bundle
                .items
                .iter()
                .map(|item| ReferencePromotionSourceV1 {
                    paper_id: item.paper_id.clone(),
                    anchor_id: item.anchor_id.clone(),
                    relation_kind: item.relation_kind.clone(),
                })
                .collect(),
            provenance_kind: plan.provenance.expect("promotion has provenance").into(),
            candidate_id: plan.candidate.id.clone(),
            candidate_revision: plan.draft.candidate_revision,
        };
        let decision = self.review(ReviewInputV1::ReferencePromotion {
            input: Box::new(input),
        })?;
        let ReviewDecisionV1::ReferencePromotion {
            result: Ok(prepared),
        } = decision
        else {
            return Err(Flow::original(AiError::TemporaryUnavailable));
        };
        let Some(sql) = &self.references else {
            return Err(Flow::Need(Need::References));
        };
        let (id, revision) = sql.apply(tx, &prepared).map_err(|error| {
            Flow::original(match error {
                ReferenceAcceptanceError::Database(error) => AiError::Database(error),
                ReferenceAcceptanceError::AnchorOwnership => {
                    AiError::InvalidInput(error.to_string())
                }
            })
        })?;
        Ok(Some(("reference_record".into(), id, revision)))
    }
}

impl LiteratureTraceService {
    pub(crate) async fn decide_analysis_candidate(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: AiReviewDecisionDraft,
    ) -> Result<AiCandidate, AiError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let candidate = draft.candidate_id.clone();
        tokio::spawn(async move {
            owner
                .decide_candidate_owned(store, key, ReviewRequest::Candidate(draft))
                .await
        })
        .await
        .map_err(|_| unsettled(&selected, &candidate))?
    }
    pub(crate) async fn decide_suggestion_item(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: research_workbench_native_sdk::candidate_review::items::AiSuggestionItemDecisionDraft,
    ) -> Result<AiCandidate, AiError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let candidate = draft.candidate_id.clone();
        tokio::spawn(async move {
            owner
                .decide_candidate_owned(store, key, ReviewRequest::Suggestion(draft))
                .await
        })
        .await
        .map_err(|_| unsettled(&selected, &candidate))?
    }
    pub(crate) async fn decide_paper_concept_item(
        self: &Arc<Self>,
        store: AppStore,
        key: String,
        draft: research_workbench_native_sdk::candidate_review::items::AiSuggestionItemDecisionDraft,
    ) -> Result<AiCandidate, AiError> {
        let owner = Arc::clone(self);
        let selected = key.clone();
        let candidate = draft.candidate_id.clone();
        tokio::spawn(async move {
            owner
                .decide_candidate_owned(store, key, ReviewRequest::Paper(draft))
                .await
        })
        .await
        .map_err(|_| unsettled(&selected, &candidate))?
    }
    async fn decide_candidate_owned(
        &self,
        store: AppStore,
        key: String,
        draft: ReviewRequest,
    ) -> Result<AiCandidate, AiError> {
        self.available().map_err(template::technical)?;
        let _lifetime = self.doi_operations.read().await;
        let mut policy = ReviewPolicy::default();
        // Domain rules and fixed resources extend the exact original observation prefix.
        loop {
            self.available().map_err(template::technical)?;
            let source = store.clone();
            let request = draft.clone();
            let (next, result) = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let mut db = source.connection().map_err(|error| Flow::original(AiError::from(error)))?;
                    guarded_transaction::run_mut(&mut db, |db| {
                        let result = match request {
                            ReviewRequest::Candidate(draft)=>decide_with(db,draft,true,&mut policy),
                            ReviewRequest::Paper(draft)=>crate::ai::paper_item_review::decide_with(db,draft,true,&mut policy),
                            ReviewRequest::Suggestion(draft)=>crate::ai::item_review::decide_with(db,draft,true,&mut policy),
                        };
                        let unknown = !db.is_autocommit() || matches!(&result, Err(Flow::Fail(failure)) if matches!(&failure.error, AiError::GenerationStorageUnknown(_)));
                        (result, unknown)
                    })
                })();
                (policy, result)
            }).await.map_err(|_| unsettled(&key, draft.candidate_id()))?;
            policy = next;
            match result {
                Ok(candidate) => return Ok(candidate),
                Err(Flow::Fail(failure)) => return Err(failure.error),
                Err(Flow::Need(Need::Decision(input))) => {
                    let (decision, origin) = self
                        .analysis_workflow_decision(&key, *input.clone())
                        .await
                        .map_err(template::technical)?;
                    policy.observations.push(Observation {
                        input: *input,
                        decision,
                        origin,
                    });
                }
                Err(Flow::Need(
                    need @ (Need::References
                    | Need::Project
                    | Need::Derived
                    | Need::ReadingUnits
                    | Need::Knowledge
                    | Need::PaperKnowledge),
                )) => {
                    let catalog = Arc::clone(&self.catalog);
                    let selected = key.clone();
                    policy = tokio::task::spawn_blocking(move || {
                        let package = package::accept_analysis_workflow(&catalog, &selected).map_err(template::technical)?;
                        match need {
                            Need::References => {
                                package::accept_reference_records(&catalog, &selected).map_err(template::technical)?;
                                policy.references = Some(ReferenceAcceptanceSql::load(&catalog, &package).map_err(|cause| template::technical(Self::error(cause.code, cause.message).package(&selected)))?);
                            },
                            Need::Project => policy.project_sql = Some(data::load_text(&catalog, &package, "literature_library", "data/candidate-review-project-contains.v1.sql", "ca10e2afb070fa6b290100644cbe2886a6ed135e3bbc129559defe58f619b736").map_err(|cause| template::technical(Self::error(cause.code, cause.message).package(&selected)))?),
                            Need::Derived => policy.derived = Some(crate::runtime_plugins::literature_derived_view::DerivedAcceptanceSql::load(&catalog, &package).map_err(|cause| template::technical(Self::error(cause.code, cause.message).package(&selected)))?),
                            Need::ReadingUnits => policy.reading_units = Some(crate::runtime_plugins::literature_reading_units::accepted::ReadingUnitAcceptanceSql::load(&catalog, &package).map_err(|cause| template::technical(Self::error(cause.code, cause.message).package(&selected)))?),
                            Need::Knowledge | Need::PaperKnowledge => {
                                package::accept_concepts(&catalog,&selected).map_err(template::technical)?;
                                policy.knowledge = Some(
                                    if matches!(need, Need::PaperKnowledge) {
                                        crate::runtime_plugins::literature_knowledge_promotion::KnowledgeSql::load_paper(&catalog, &package)
                                    } else {
                                        crate::runtime_plugins::literature_knowledge_promotion::KnowledgeSql::load(&catalog, &package)
                                    }
                                    .map_err(|cause| template::technical(
                                        Self::error(cause.code, cause.message).package(&selected)
                                    ))?
                                );
                                policy.semantic=Some(crate::runtime_plugins::literature_concepts::ConceptResources::load(&catalog,&package).map_err(|cause|template::technical(Self::error(cause.code,cause.message).package(&selected)))?);
                            },
                            Need::Decision(_) => unreachable!("resource request"),
                        }
                        Ok::<_, AiError>(policy)
                    }).await.map_err(|_| unsettled(&key, draft.candidate_id()))??;
                }
            }
        }
    }
}
fn unsettled(key: &str, candidate: &str) -> AiError {
    template::technical(
        LiteratureTraceService::error(
            "plugin_host_source_worker_unsettled",
            format!("候选审阅结果尚未确认，请核对原候选 {candidate} 与审阅幂等键。"),
        )
        .package(key),
    )
}
