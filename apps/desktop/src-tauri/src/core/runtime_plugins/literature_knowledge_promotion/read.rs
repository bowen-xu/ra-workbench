//! Original knowledge read-back, also used after human capture commits.
use super::*;
use rusqlite::Connection;
pub(super) struct KnowledgeRead<'a> {
    pub db: &'a Connection,
    pub sql: &'a KnowledgeSql,
}
impl KnowledgeRead<'_> {
    pub(super) fn resolve_active_concept_id<P>(&self, concept_id: &str) -> Result<String, P> {
        let canonical = self
            .db
            .query_row(
                self.sql
                    .statement("knowledge-promotion-canonical-concept.v1.sql"),
                [concept_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(canonical.unwrap_or_else(|| concept_id.to_owned()))
    }
    pub(super) fn term_from<P>(&self, id: &str) -> Result<Term, P> {
        self.db
            .query_row(
                self.sql.statement("nal-bootstrap-term-read.v1.sql"),
                [id],
                |row| {
                    Ok(Term {
                        id: row.get(0)?,
                        text: row.get(1)?,
                        normalized_text: row.get(2)?,
                        language: row.get(3)?,
                        provenance_kind: row.get(4)?,
                        revision: row.get(5)?,
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| PromotionError::from(KnowledgeError::NotFound))
    }

    pub(super) fn concept_from<P>(&self, id: &str) -> Result<Concept, P> {
        let values = self
            .db
            .query_row(
                self.sql.statement("nal-bootstrap-concept-main.v1.sql"),
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| PromotionError::from(KnowledgeError::NotFound))?;
        let term_rows = self
            .db
            .prepare(self.sql.statement("nal-bootstrap-concept-terms.v1.sql"))?
            .query_map([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
        let preferred_id = term_rows
            .iter()
            .find(|(_, relation)| relation == "preferred")
            .map(|(term_id, _)| term_id)
            .ok_or_else(|| KnowledgeError::InvalidInput("概念缺少首选语词".to_owned()))?;
        let terms = term_rows
            .iter()
            .map(|(term_id, _)| self.term_from(term_id))
            .collect::<Result<Vec<_>, _>>()?;
        let preferred_term = self.term_from(preferred_id)?;
        Ok(Concept {
            id: id.to_owned(),
            preferred_term,
            terms,
            label: values.0,
            explanation: values.1,
            disambiguation: values.2,
            domain: values.3,
            tradition: values.4,
            period: values.5,
            provenance_kind: values.6,
            revision: values.7,
            definition_count: values.8,
            source_count: values.9,
            created_at: values.10,
            updated_at: values.11,
        })
    }

    pub(super) fn term_usage_from<P>(&self, id: &str) -> Result<TermUsage, P> {
        let values = self
            .db
            .query_row(
                self.sql
                    .statement("knowledge-promotion-term-usage-read.v1.sql"),
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, i64>(13)?,
                        row.get::<_, String>(14)?,
                        row.get::<_, String>(15)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| PromotionError::from(KnowledgeError::NotFound))?;
        Ok(TermUsage {
            id: values.0,
            term: self.term_from(&values.1)?,
            concept_id: values.2,
            anchor_id: values.3,
            paper_id: values.4,
            paper_title: values.5,
            attachment_id: values.6,
            content_sha256: values.7,
            page_index: values.8,
            page_label: values.9,
            quote_exact: values.10,
            usage_text: values.11,
            provenance_kind: values.12,
            revision: values.13,
            created_at: values.14,
            updated_at: values.15,
        })
    }

    pub(super) fn definition_claim_from<P>(&self, id: &str) -> Result<DefinitionClaim, P> {
        self.db
            .query_row(
                self.sql
                    .statement("knowledge-promotion-definition-read.v1.sql"),
                [id],
                |row| {
                    Ok(DefinitionClaim {
                        id: row.get(0)?,
                        concept_id: row.get(1)?,
                        statement: row.get(2)?,
                        scope_note: row.get(3)?,
                        provenance_kind: row.get(4)?,
                        epistemic_role: row.get(5)?,
                        revision: row.get(6)?,
                        evidence_count: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| PromotionError::from(KnowledgeError::NotFound))
    }
}
