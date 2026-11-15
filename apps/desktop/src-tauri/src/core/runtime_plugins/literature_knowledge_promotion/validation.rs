//! Original validation boundaries, reentered through finite Native decisions.
use super::*;
use research_workbench_native_sdk::concepts::ConceptTermPlanV1;
impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(super) fn decide(&mut self, input: Input) -> Result<Decision, P::Pause> {
        let ordinal = self.ordinal;
        self.ordinal += 1;
        self.policy.decide(ordinal, input)
    }
    pub(super) fn paper_preparation(
        &mut self,
        draft: &research_workbench_native_sdk::knowledge_promotion::paper::AiPaperConceptPromotion,
    ) -> Result<
        Box<research_workbench_native_sdk::knowledge_promotion::paper::PaperConceptPreparationV1>,
        P::Pause,
    > {
        match self.decide(Input::PaperConcept {
            draft: Box::new(draft.clone()),
        })? {
            Decision::PaperConcept { result } => {
                result.map_err(|message| KnowledgeError::InvalidInput(message).into())
            }
            _ => Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        }
    }
    pub(super) fn checked(&mut self, input: Input) -> Result<(), P::Pause> {
        checked_decision(self.decide(input)?)
    }
    pub(super) fn term_plan(
        &mut self,
        text: &str,
        language: &str,
    ) -> Result<ConceptTermPlanV1, P::Pause> {
        match self.decide(Input::Term {
            text: text.into(),
            language: language.into(),
        })? {
            Decision::Term { prepared } => match prepared.rejection {
                Some(reason) => Err(KnowledgeError::InvalidInput(reason.message().into()).into()),
                None => Ok(prepared),
            },
            _ => Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        }
    }
    pub(super) fn details(
        &mut self,
        details: &NewConceptDetails,
    ) -> Result<(NewConceptDetails, String), P::Pause> {
        match self.decide(Input::Details {
            details: details.clone(),
        })? {
            Decision::Details { result } => {
                result.map_err(|message| KnowledgeError::InvalidInput(message).into())
            }
            _ => Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        }
    }
    pub(super) fn definition(
        &mut self,
        statement: &str,
        scope_note: &str,
    ) -> Result<(String, String), P::Pause> {
        match self.decide(Input::Definition {
            statement: statement.into(),
            scope_note: scope_note.into(),
        })? {
            Decision::Definition { result } => {
                result.map_err(|message| KnowledgeError::InvalidInput(message).into())
            }
            _ => Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        }
    }
    pub(super) fn name_available(&mut self, name: &str) -> Result<(), P::Pause> {
        let collision = self.db.query_row(
            self.sql
                .statement("nal-bootstrap-concept-name-available.v1.sql"),
            params![name, Option::<&str>::None],
            |row| row.get(0),
        )?;
        self.checked(Input::NameCollision { collision })
    }
    pub(super) fn validate_origin_and_anchors(
        &mut self,
        provenance_kind: &str,
        anchor_ids: &[String],
    ) -> Result<(), P::Pause> {
        self.checked(Input::Origin {
            provenance_kind: provenance_kind.into(),
            anchor_ids: anchor_ids.to_vec(),
        })?;
        let mut unique = std::collections::HashSet::new();
        for anchor in anchor_ids {
            self.checked(Input::UniqueAnchor {
                duplicate: !unique.insert(anchor),
            })?;
            let exists = self.db.query_row(
                self.sql
                    .statement("knowledge-promotion-anchor-exists.v1.sql"),
                [anchor],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                return Err(KnowledgeError::NotFound.into());
            }
        }
        Ok(())
    }
    pub(super) fn relation_node(&mut self, id: &str) -> Result<RelationNodeSummary, P::Pause> {
        self.checked(Input::Node { id: id.into() })?;
        self.db
            .query_row(
                self.sql
                    .statement("knowledge-promotion-relation-concept-node.v1.sql"),
                [id],
                |row| {
                    Ok(RelationNodeSummary {
                        kind: "concept".into(),
                        id: row.get(0)?,
                        label: row.get(1)?,
                        detail: row.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| KnowledgeError::NotFound.into())
    }
}

pub(super) fn checked_decision<P>(decision: Decision) -> Result<(), P> {
    match decision {
        Decision::Checked { rejection: None } => Ok(()),
        Decision::Checked {
            rejection: Some(message),
        } => Err(KnowledgeError::InvalidInput(message).into()),
        _ => Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
    }
}
