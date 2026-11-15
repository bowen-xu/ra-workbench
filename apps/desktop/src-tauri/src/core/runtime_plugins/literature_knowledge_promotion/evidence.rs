//! Original knowledge storage sequence on the caller-owned review transaction.
use super::*;
impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(super) fn term_usage_from(&mut self, id: &str) -> Result<TermUsage, P::Pause> {
        KnowledgeRead {
            db: self.db,
            sql: self.sql,
        }
        .term_usage_from(id)
    }

    pub(super) fn definition_claim_from(&mut self, id: &str) -> Result<DefinitionClaim, P::Pause> {
        KnowledgeRead {
            db: self.db,
            sql: self.sql,
        }
        .definition_claim_from(id)
    }

    pub(super) fn insert_definition_claim_with_origin(
        &mut self,
        concept_id: &str,
        statement: &str,
        scope_note: &str,
        anchor_ids: &[String],
        origin: DefinitionClaimOrigin<'_>,
    ) -> Result<String, P::Pause> {
        let (statement, scope_note) = self.definition(statement, scope_note)?;
        self.insert_prepared_definition(concept_id, &statement, &scope_note, anchor_ids, origin)
    }

    pub(super) fn insert_prepared_definition(
        &mut self,
        concept_id: &str,
        statement: &str,
        scope_note: &str,
        anchor_ids: &[String],
        origin: DefinitionClaimOrigin<'_>,
    ) -> Result<String, P::Pause> {
        let id = Uuid::new_v4().to_string();
        self.db.execute(
            self.sql
                .statement("knowledge-promotion-definition-insert.v1.sql"),
            params![
                id,
                concept_id,
                statement,
                scope_note.trim(),
                origin.provenance_kind
            ],
        )?;
        for anchor_id in anchor_ids {
            self.db.execute(
                self.sql
                    .statement("knowledge-promotion-definition-evidence-insert.v1.sql"),
                params![Uuid::new_v4().to_string(), id, anchor_id],
            )?;
        }
        let claim = self.definition_claim_from(&id)?;
        self.save_revision(
            "definition_claim",
            &id,
            1,
            &serde_json::to_value(&claim)
                .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?,
            origin.editor_kind,
            origin.reason,
        )?;
        Ok(id)
    }

    pub(super) fn save_revision(
        &mut self,
        object_kind: &str,
        object_id: &str,
        revision: i64,
        snapshot: &serde_json::Value,
        editor_kind: &str,
        change_reason: &str,
    ) -> Result<(), P::Pause> {
        self.db.execute(
            self.sql
                .statement("nal-bootstrap-research-revision-insert.v1.sql"),
            params![
                Uuid::new_v4().to_string(),
                object_kind,
                object_id,
                revision,
                serde_json::to_string(snapshot)
                    .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?,
                editor_kind,
                change_reason
            ],
        )?;
        Ok(())
    }
}
