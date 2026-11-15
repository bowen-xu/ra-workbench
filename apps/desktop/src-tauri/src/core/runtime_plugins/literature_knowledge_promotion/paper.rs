//! Original paper roles, surface forms and definitions on the same review transaction.
use super::*;
use research_workbench_native_sdk::knowledge_promotion::paper::*;
impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(super) fn promote_ai_paper_concept(
        &mut self,
        draft: AiPaperConceptPromotion,
        provenance_kind: &str,
    ) -> Result<AiAcceptedKnowledgeTarget, P::Pause> {
        let prepared = self.paper_preparation(&draft)?;
        let target = self.promote_ai_concept(prepared.base, provenance_kind)?;
        let (concept_id, term_usage_id) = if target.kind == "concept" {
            self.attach_paper_surface_forms(&target.id, &prepared.surface_forms, provenance_kind)?;
            self.attach_paper_definitions(
                &target.id,
                &prepared.definition_claims,
                provenance_kind,
            )?;
            (Some(target.id.as_str()), None)
        } else {
            (None, Some(target.id.as_str()))
        };
        let assertion_id = Uuid::new_v4().to_string();
        self.db.execute(
            self.sql.statement("paper-concept-assertion-insert.v1.sql"),
            params![assertion_id, draft.paper_id, concept_id, term_usage_id],
        )?;
        self.db.execute(
            self.sql
                .statement("paper-concept-assertion-revision-insert.v1.sql"),
            params![
                Uuid::new_v4().to_string(),
                assertion_id,
                draft.paper_function,
                draft.novelty_status,
                draft.explanation_status,
                draft.explanation,
                draft.role_rationale,
                draft.novelty_rationale,
                draft.source_candidate_id,
                draft.source_candidate_revision,
                draft.source_local_concept_id
            ],
        )?;
        Ok(target)
    }
    fn attach_paper_surface_forms(
        &mut self,
        concept_id: &str,
        forms: &[AiPaperSurfaceFormPromotion],
        provenance_kind: &str,
    ) -> Result<(), P::Pause> {
        for form in forms {
            let term = self.ensure_term_with_origin(
                &form.text,
                form.language.as_deref().unwrap_or(""),
                provenance_kind,
                "ai_review",
                "paper_concept_acceptance",
            )?;
            self.attach_alias_to_concept(
                concept_id,
                &term.id,
                "ai_review",
                "paper_concept_surface_form",
            )?;
            for anchor_id in &form.anchor_ids {
                self.ensure_term_usage(
                    &term.id,
                    Some(concept_id),
                    anchor_id,
                    &form.text,
                    provenance_kind,
                )?;
            }
        }
        Ok(())
    }
    fn attach_paper_definitions(
        &mut self,
        concept_id: &str,
        definitions: &[AiPaperDefinitionPromotion],
        provenance_kind: &str,
    ) -> Result<(), P::Pause> {
        for definition in definitions {
            self.promote_ai_definition(
                AiDefinitionPromotion {
                    concept_id: concept_id.to_owned(),
                    statement: definition.statement.clone(),
                    scope_note: Some(definition.scope_note.clone()),
                    anchor_ids: definition.anchor_ids.clone(),
                },
                provenance_kind,
            )?;
        }
        Ok(())
    }
}
