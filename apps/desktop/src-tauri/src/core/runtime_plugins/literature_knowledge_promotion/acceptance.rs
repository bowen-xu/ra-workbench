//! Original knowledge storage sequence on the caller-owned review transaction.
use super::*;
impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(super) fn promote_ai_concept(
        &mut self,
        draft: AiConceptPromotion,
        provenance_kind: &str,
    ) -> Result<AiAcceptedKnowledgeTarget, P::Pause> {
        self.validate_origin_and_anchors(provenance_kind, &draft.anchor_ids)?;
        let term = self.ensure_term_with_origin(
            &draft.term,
            draft.language.as_deref().unwrap_or(""),
            provenance_kind,
            "ai_review",
            "ai_suggestion_acceptance",
        )?;
        let concept_id = match draft.resolution {
            AiConceptResolution::CreateNew => Some(self.insert_concept_with_origin(
                &term.id,
                &NewConceptDetails {
                    label: draft.term.clone(),
                    explanation: draft.explanation.clone(),
                    disambiguation: draft.disambiguation.unwrap_or_default(),
                    domain: draft.domain.unwrap_or_default(),
                    tradition: draft.tradition.unwrap_or_default(),
                    period: draft.period.unwrap_or_default(),
                    change_reason: Some("ai_suggestion_acceptance".to_owned()),
                },
                provenance_kind,
                "ai_review",
                "ai_suggestion_acceptance",
            )?),
            AiConceptResolution::LinkExisting(concept_id) => {
                let concept_id = self.resolve_active_concept_id(&concept_id)?;
                self.attach_alias_to_concept(
                    &concept_id,
                    &term.id,
                    "ai_review",
                    "ai_suggestion_alias",
                )?;
                Some(concept_id)
            }
            AiConceptResolution::Unresolved => None,
        };
        let mut usage_ids = Vec::new();
        for anchor_id in &draft.anchor_ids {
            usage_ids.push(self.ensure_term_usage(
                &term.id,
                concept_id.as_deref(),
                anchor_id,
                &draft.explanation,
                provenance_kind,
            )?);
        }
        if let Some(concept_id) = concept_id {
            let concept = self.concept_from(&concept_id)?;
            Ok(AiAcceptedKnowledgeTarget {
                kind: "concept",
                id: concept.id,
                revision: concept.revision,
            })
        } else {
            let usage = self.term_usage_from(
                usage_ids
                    .first()
                    .ok_or_else(|| KnowledgeError::InvalidInput("概念建议缺少来源".to_owned()))?,
            )?;
            Ok(AiAcceptedKnowledgeTarget {
                kind: "term_usage",
                id: usage.id,
                revision: usage.revision,
            })
        }
    }

    pub(super) fn promote_ai_definition(
        &mut self,
        draft: AiDefinitionPromotion,
        provenance_kind: &str,
    ) -> Result<AiAcceptedKnowledgeTarget, P::Pause> {
        self.validate_origin_and_anchors(provenance_kind, &draft.anchor_ids)?;
        let concept_id = self.resolve_active_concept_id(&draft.concept_id)?;
        self.concept_from(&concept_id)?;
        let id = self.insert_definition_claim_with_origin(
            &concept_id,
            &draft.statement,
            draft.scope_note.as_deref().unwrap_or(""),
            &draft.anchor_ids,
            DefinitionClaimOrigin {
                provenance_kind,
                editor_kind: "ai_review",
                reason: "ai_suggestion_acceptance",
            },
        )?;
        let claim = self.definition_claim_from(&id)?;
        Ok(AiAcceptedKnowledgeTarget {
            kind: "definition_claim",
            id,
            revision: claim.revision,
        })
    }

    pub(super) fn promote_ai_relation(
        &mut self,
        draft: AiRelationPromotion,
        provenance_kind: &str,
    ) -> Result<AiAcceptedKnowledgeTarget, P::Pause> {
        self.validate_origin_and_anchors(provenance_kind, &draft.anchor_ids)?;
        self.checked(Input::Predicate {
            predicate_kind: draft.predicate_kind.clone(),
        })?;
        let subject_id = self.resolve_active_concept_id(&draft.subject_concept_id)?;
        let object_id = self.resolve_active_concept_id(&draft.object_concept_id)?;
        self.checked(Input::DifferentConcepts {
            same: subject_id == object_id,
        })?;
        if let Some(project_id) = draft.project_id.as_deref() {
            let exists = self.db.query_row(
                self.sql
                    .statement("knowledge-promotion-project-exists.v1.sql"),
                [project_id],
                |row| row.get::<_, bool>(0),
            )?;
            if !exists {
                return Err(KnowledgeError::NotFound.into());
            }
        }
        let subject = self.relation_node(&subject_id)?;
        let object = self.relation_node(&object_id)?;
        let id = Uuid::new_v4().to_string();
        let scope_note = draft.scope_note.as_deref().unwrap_or("").trim().to_owned();
        self.db.execute(
            self.sql
                .statement("knowledge-promotion-relation-insert.v1.sql"),
            params![
                id,
                draft.project_id,
                subject.id,
                subject.label,
                draft.predicate_kind,
                object.id,
                object.label,
                scope_note,
                provenance_kind,
            ],
        )?;
        for anchor_id in &draft.anchor_ids {
            self.db.execute(
                self.sql
                    .statement("knowledge-promotion-relation-evidence-insert.v1.sql"),
                params![Uuid::new_v4().to_string(), id, anchor_id],
            )?;
        }
        self.save_revision(
            "relation_assertion",
            &id,
            1,
            &serde_json::json!({
                "id": id,
                "projectId": draft.project_id,
                "subject": subject,
                "predicateKind": draft.predicate_kind,
                "object": object,
                "scopeNote": scope_note,
                "provenanceKind": provenance_kind,
                "epistemicRole": "assertion",
                "revision": 1
            }),
            "ai_review",
            "ai_suggestion_acceptance",
        )?;
        Ok(AiAcceptedKnowledgeTarget {
            kind: "relation_assertion",
            id,
            revision: 1,
        })
    }

    pub(crate) fn ensure_term_usage(
        &mut self,
        term_id: &str,
        concept_id: Option<&str>,
        anchor_id: &str,
        usage_text: &str,
        provenance_kind: &str,
    ) -> Result<String, P::Pause> {
        self.ensure_term_usage_with_origin(
            term_id,
            concept_id,
            anchor_id,
            usage_text.trim(),
            DefinitionClaimOrigin {
                provenance_kind,
                editor_kind: "ai_review",
                reason: "ai_suggestion_acceptance",
            },
        )
    }

    pub(super) fn ensure_term_usage_with_origin(
        &mut self,
        term_id: &str,
        concept_id: Option<&str>,
        anchor_id: &str,
        usage_text: &str,
        origin: DefinitionClaimOrigin<'_>,
    ) -> Result<String, P::Pause> {
        let existing = self
            .db
            .query_row(
                self.sql
                    .statement("knowledge-promotion-term-usage-lookup.v1.sql"),
                params![term_id, anchor_id, concept_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        self.db.execute(
            self.sql
                .statement("knowledge-promotion-term-usage-insert.v1.sql"),
            params![
                id,
                term_id,
                concept_id,
                anchor_id,
                usage_text,
                origin.provenance_kind
            ],
        )?;
        let usage = self.term_usage_from(&id)?;
        self.save_revision(
            "term_usage",
            &id,
            1,
            &serde_json::to_value(&usage)
                .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?,
            origin.editor_kind,
            origin.reason,
        )?;
        Ok(id)
    }
}
