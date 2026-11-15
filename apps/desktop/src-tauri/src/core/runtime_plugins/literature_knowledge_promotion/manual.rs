//! Original manual definition writes and revision reads on the existing Core store.
use super::*;
use research_workbench_native_sdk::knowledge_promotion::manual::{
    DefinitionClaimDraft, DefinitionRevisionRejectionV1, ResearchObjectRevision,
};
use rusqlite::Connection;
impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(crate) fn save_definition(
        &mut self,
        draft: &DefinitionClaimDraft,
    ) -> Result<String, P::Pause> {
        let current_revision = match &draft.id {
            Some(id) => Some(
                self.db
                    .query_row(
                        self.sql
                            .statement("definition-manual-current-revision.v1.sql"),
                        [id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?
                    .ok_or(KnowledgeError::NotFound)?,
            ),
            None => None,
        };
        match self.decide(Input::DefinitionRevision {
            expected_revision: draft.expected_revision,
            current_revision,
        })? {
            Decision::DefinitionRevision { rejection: None } => {}
            Decision::DefinitionRevision {
                rejection: Some(DefinitionRevisionRejectionV1::NewExpectedRevision),
            } => {
                return Err(KnowledgeError::InvalidInput(
                    "新建定义不能指定 expectedRevision".into(),
                )
                .into());
            }
            Decision::DefinitionRevision {
                rejection: Some(DefinitionRevisionRejectionV1::Conflict { current_revision }),
            } => return Err(KnowledgeError::RevisionConflict { current_revision }.into()),
            _ => return Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        }
        let concept_id = if draft.id.is_some() {
            draft.concept_id.clone()
        } else {
            self.resolve_active_concept_id(&draft.concept_id)?
        };
        let statement = draft.statement.trim();
        let scope_note = draft.scope_note.trim();
        self.checked(Input::ManualDefinition {
            concept_identity_sha256: crate::runtime_plugins::files::digest(concept_id.as_bytes()),
            statement_is_empty: statement.is_empty(),
        })?;
        let Some(id) = &draft.id else {
            let anchors = draft.anchor_id.iter().cloned().collect::<Vec<_>>();
            return self.insert_prepared_definition(
                &concept_id,
                statement,
                scope_note,
                &anchors,
                DefinitionClaimOrigin {
                    provenance_kind: "human_original",
                    editor_kind: "human",
                    reason: draft.change_reason.as_deref().unwrap_or("manual_create"),
                },
            );
        };
        let next_revision =
            current_revision.expect("an existing definition has its original revision") + 1;
        self.db.execute(
            self.sql.statement("definition-manual-update.v1.sql"),
            params![id, concept_id, statement, scope_note, next_revision],
        )?;
        if let Some(anchor_id) = &draft.anchor_id {
            self.db.execute(
                self.sql
                    .statement("knowledge-promotion-definition-evidence-insert.v1.sql"),
                params![Uuid::new_v4().to_string(), id, anchor_id],
            )?;
        }
        let claim = self.definition_claim_from(id)?;
        self.save_revision(
            "definition_claim",
            id,
            claim.revision,
            &serde_json::to_value(&claim)
                .map_err(|e| KnowledgeError::InvalidInput(e.to_string()))?,
            "human",
            draft.change_reason.as_deref().unwrap_or("manual_edit"),
        )?;
        Ok(id.clone())
    }
}

pub(crate) fn read_definitions<P>(
    db: &Connection,
    sql: &KnowledgeSql,
    concept_id: &str,
) -> Result<Vec<DefinitionClaim>, P> {
    let reader = KnowledgeRead { db, sql };
    let canonical = reader.resolve_active_concept_id(concept_id)?;
    let ids = db
        .prepare(sql.statement("definition-manual-list.v1.sql"))?
        .query_map([canonical], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ids.iter()
        .map(|id| reader.definition_claim_from(id))
        .collect()
}

pub(crate) fn read_definition<P>(
    db: &Connection,
    sql: &KnowledgeSql,
    id: &str,
) -> Result<DefinitionClaim, P> {
    KnowledgeRead { db, sql }.definition_claim_from(id)
}

pub(crate) fn read_revisions(
    db: &Connection,
    sql: &KnowledgeSql,
    object_kind: &str,
    object_id: &str,
) -> std::result::Result<Vec<ResearchObjectRevision>, KnowledgeError> {
    Ok(db
        .prepare(sql.statement("knowledge-manual-revisions.v1.sql"))?
        .query_map(params![object_kind, object_id], |row| {
            let snapshot_json: String = row.get(4)?;
            Ok(ResearchObjectRevision {
                id: row.get(0)?,
                object_kind: row.get(1)?,
                object_id: row.get(2)?,
                revision: row.get(3)?,
                snapshot: serde_json::from_str(&snapshot_json)
                    .unwrap_or_else(|_| serde_json::json!({"decodeError":true})),
                editor_kind: row.get(5)?,
                change_reason: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}
