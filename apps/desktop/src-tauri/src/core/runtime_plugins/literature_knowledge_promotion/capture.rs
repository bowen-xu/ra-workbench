//! Original human capture sequence on the caller's Deferred transaction.
use super::*;
use research_workbench_native_sdk::knowledge_promotion::capture::*;
use rusqlite::Connection;

pub(crate) struct CaptureIds {
    term: String,
    concept: Option<String>,
    usage: String,
    definition: Option<String>,
}

impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(crate) fn capture(&mut self, draft: &ConceptCaptureDraft) -> Result<CaptureIds, P::Pause> {
        let quote = self
            .db
            .query_row(
                self.sql.statement("concept-capture-anchor-quote.v1.sql"),
                [&draft.anchor_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or(KnowledgeError::NotFound)?;
        let prepared = match self.decide(Input::Capture {
            draft: Box::new(draft.clone()),
            quote,
        })? {
            Decision::Capture { prepared } => prepared,
            _ => return Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        };
        let term = self.ensure_term_with_origin(
            &draft.term_text,
            draft.language.as_deref().unwrap_or(""),
            "human_original",
            "human",
            "manual_create",
        )?;
        let (concept, definition_statement) = match prepared.resolution {
            ConceptResolutionDraft::CreateNew {
                concept,
                definition_statement,
            } => {
                let id = self.insert_concept_with_origin(
                    &term.id,
                    &concept,
                    "human_original",
                    "human",
                    concept
                        .change_reason
                        .as_deref()
                        .unwrap_or("selection_capture"),
                )?;
                (Some(id), definition_statement)
            }
            ConceptResolutionDraft::LinkExisting {
                concept_id,
                definition_statement,
            } => {
                let id = self.resolve_active_concept_id(&concept_id)?;
                self.attach_alias_to_concept(&id, &term.id, "human", "selection_capture_alias")?;
                (Some(id), definition_statement)
            }
            ConceptResolutionDraft::Unresolved => (None, None),
        };
        let definition = match (concept.as_deref(), definition_statement) {
            (Some(id), Some(statement)) => Some(self.insert_definition_claim_with_origin(
                id,
                &statement,
                "",
                std::slice::from_ref(&draft.anchor_id),
                DefinitionClaimOrigin {
                    provenance_kind: "human_original",
                    editor_kind: "human",
                    reason: "selection_capture",
                },
            )?),
            _ => None,
        };
        let usage = self.ensure_term_usage_with_origin(
            &term.id,
            concept.as_deref(),
            &draft.anchor_id,
            &prepared.usage_text,
            DefinitionClaimOrigin {
                provenance_kind: "human_original",
                editor_kind: "human",
                reason: "selection_capture",
            },
        )?;
        Ok(CaptureIds {
            term: term.id,
            concept,
            usage,
            definition,
        })
    }
}

pub(crate) fn read_capture<P>(
    db: &Connection,
    sql: &KnowledgeSql,
    ids: &CaptureIds,
) -> Result<ConceptCaptureResult, P> {
    let reader = KnowledgeRead { db, sql };
    Ok(ConceptCaptureResult {
        term: reader.term_from(&ids.term)?,
        concept: ids
            .concept
            .as_deref()
            .map(|id| reader.concept_from(id))
            .transpose()?,
        usage: reader.term_usage_from(&ids.usage)?,
        definition_claim: ids
            .definition
            .as_deref()
            .map(|id| reader.definition_claim_from(id))
            .transpose()?,
    })
}
