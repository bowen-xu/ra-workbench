//! Original knowledge storage sequence on the caller-owned review transaction.
use super::*;
impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(super) fn term_from(&mut self, id: &str) -> Result<Term, P::Pause> {
        KnowledgeRead {
            db: self.db,
            sql: self.sql,
        }
        .term_from(id)
    }

    pub(super) fn ensure_term_with_origin(
        &mut self,
        text: &str,
        language: &str,
        provenance_kind: &str,
        editor_kind: &str,
        reason: &str,
    ) -> Result<Term, P::Pause> {
        let prepared = self.term_plan(text, language)?;
        let text = prepared.text;
        let language = prepared.language;
        let normalized = prepared.normalized_text;
        let existing_id = self
            .db
            .query_row(
                self.sql.statement("nal-bootstrap-term-lookup.v1.sql"),
                params![normalized, language],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(id) = existing_id {
            return self.term_from(&id);
        }

        let id = Uuid::new_v4().to_string();
        self.db.execute(
            self.sql.statement("nal-bootstrap-term-insert.v1.sql"),
            params![id, text, normalized, language, provenance_kind],
        )?;
        let term = self.term_from(&id)?;
        self.save_revision(
            "term",
            &id,
            1,
            &serde_json::to_value(&term)
                .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?,
            editor_kind,
            reason,
        )?;
        Ok(term)
    }

    pub(super) fn concept_from(&mut self, id: &str) -> Result<Concept, P::Pause> {
        KnowledgeRead {
            db: self.db,
            sql: self.sql,
        }
        .concept_from(id)
    }

    pub(super) fn insert_concept_with_origin(
        &mut self,
        term_id: &str,
        details: &NewConceptDetails,
        provenance_kind: &str,
        editor_kind: &str,
        reason: &str,
    ) -> Result<String, P::Pause> {
        let (normalized, name) = self.details(details)?;
        self.name_available(&name)?;
        let id = Uuid::new_v4().to_string();
        self.db.execute(
            self.sql.statement("nal-bootstrap-concept-insert.v1.sql"),
            params![
                id,
                normalized.label,
                normalized.explanation,
                normalized.disambiguation,
                normalized.domain,
                normalized.tradition,
                normalized.period,
                provenance_kind
            ],
        )?;
        self.db.execute(
            self.sql
                .statement("nal-bootstrap-concept-preferred-insert.v1.sql"),
            params![id, term_id],
        )?;
        let concept = self.concept_from(&id)?;
        self.save_revision(
            "concept",
            &id,
            1,
            &serde_json::to_value(&concept)
                .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?,
            editor_kind,
            reason,
        )?;
        self.policy.register_semantic(self.db, &concept)?;
        Ok(id)
    }

    pub(super) fn attach_alias_to_concept(
        &mut self,
        concept_id: &str,
        term_id: &str,
        editor_kind: &str,
        reason: &str,
    ) -> Result<Concept, P::Pause> {
        let inserted = self.db.execute(
            self.sql
                .statement("knowledge-promotion-alias-insert.v1.sql"),
            params![concept_id, term_id],
        )?;
        if inserted == 0 {
            return self.concept_from(concept_id);
        }
        self.db.execute(
            self.sql
                .statement("knowledge-promotion-concept-revision-update.v1.sql"),
            [concept_id],
        )?;
        let concept = self.concept_from(concept_id)?;
        self.save_revision(
            "concept",
            concept_id,
            concept.revision,
            &serde_json::to_value(&concept)
                .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?,
            editor_kind,
            reason,
        )?;
        self.policy.register_semantic(self.db, &concept)?;
        Ok(concept)
    }

    pub(super) fn resolve_active_concept_id(
        &mut self,
        concept_id: &str,
    ) -> Result<String, P::Pause> {
        KnowledgeRead {
            db: self.db,
            sql: self.sql,
        }
        .resolve_active_concept_id(concept_id)
    }
}
