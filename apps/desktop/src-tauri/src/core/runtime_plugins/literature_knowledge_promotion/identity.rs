//! Original identity reads and transaction bodies. Native owns the qualification rules.
use super::validation::checked_decision;
use super::*;
use research_workbench_native_sdk::knowledge_promotion::{
    identity::*, manual::DefinitionRevisionRejectionV1,
};
use rusqlite::Connection;
use std::collections::{BTreeSet, HashSet};
mod storage;
use storage::{affected_relations, identity_summary, insert_event, insert_relation_impacts};
pub(crate) use storage::{identity_event_from, list_events};

pub(crate) fn precheck<P: KnowledgePolicy>(
    policy: &mut P,
    request: &IdentityRequestV1,
) -> Result<(), P::Pause> {
    let input = match request {
        IdentityRequestV1::Merge { draft } => Input::IdentityMergeRationale {
            characters: draft.rationale.chars().count(),
        },
        IdentityRequestV1::Split { draft } => split_shape(draft),
        _ => return Ok(()),
    };
    checked_decision(policy.decide(0, input)?)
}
fn split_shape(draft: &ConceptSplitDraft) -> Input {
    Input::IdentitySplitShape {
        source_count: draft.source_concept_ids.len(),
        rationale_characters: draft.rationale.chars().count(),
        duplicate_sources: draft
            .source_concept_ids
            .iter()
            .collect::<HashSet<_>>()
            .len()
            != draft.source_concept_ids.len(),
    }
}
fn digest<P>(value: &impl serde::Serialize) -> Result<[u8; 32], P> {
    serde_json::to_vec(value)
        .map(|bytes| crate::runtime_plugins::files::digest(&bytes))
        .map_err(|e| KnowledgeError::InvalidInput(e.to_string()).into())
}
pub(crate) fn preview_merge<P: KnowledgePolicy>(
    db: &Connection,
    sql: &KnowledgeSql,
    policy: &mut P,
    draft: &ConceptMergeDraft,
) -> Result<ConceptMergeImpact, P::Pause> {
    IdentityRead {
        db,
        sql,
        policy,
        ordinal: &mut 0,
    }
    .preview(draft)
}
struct IdentityRead<'a, P: KnowledgePolicy> {
    db: &'a Connection,
    sql: &'a KnowledgeSql,
    policy: &'a mut P,
    ordinal: &'a mut usize,
}
impl<P: KnowledgePolicy> IdentityRead<'_, P> {
    fn decide(&mut self, input: Input) -> Result<Decision, P::Pause> {
        let ordinal = *self.ordinal;
        *self.ordinal += 1;
        self.policy.decide(ordinal, input)
    }
    fn checked(&mut self, input: Input) -> Result<(), P::Pause> {
        checked_decision(self.decide(input)?)
    }
    fn revision(&mut self, current: i64, expected: i64) -> Result<(), P::Pause> {
        match self.decide(Input::DefinitionRevision {
            current_revision: Some(current),
            expected_revision: Some(expected),
        })? {
            Decision::DefinitionRevision { rejection: None } => Ok(()),
            Decision::DefinitionRevision {
                rejection: Some(DefinitionRevisionRejectionV1::Conflict { current_revision }),
            } => Err(KnowledgeError::RevisionConflict { current_revision }.into()),
            _ => Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        }
    }
    fn target(&mut self, id: &str, source: bool) -> Result<(), P::Pause> {
        let summary = identity_summary::<P::Pause>(self.db, self.sql, id)?;
        let mapped = self.db.query_row(
            self.sql.statement("concept-identity-mapping-exists.v1.sql"),
            [id],
            |r| r.get::<_, bool>(0),
        )?;
        let has_children = source
            && self.db.query_row(
                self.sql.statement("concept-identity-children-exist.v1.sql"),
                [id],
                |r| r.get::<_, bool>(0),
            )?;
        self.checked(Input::IdentityTarget {
            snapshot_sha256: digest(&summary)?,
            mapped,
            has_children,
        })
    }
    fn preview(&mut self, draft: &ConceptMergeDraft) -> Result<ConceptMergeImpact, P::Pause> {
        let unique = draft
            .sources
            .iter()
            .map(|s| s.id.as_str())
            .collect::<HashSet<_>>();
        self.checked(Input::IdentityMergeShape {
            source_count: draft.sources.len(),
            survivor_is_empty: draft.survivor_id.trim().is_empty(),
            duplicate_sources: unique.len() != draft.sources.len(),
            includes_survivor: unique.contains(draft.survivor_id.as_str()),
        })?;
        self.target(&draft.survivor_id, false)?;
        let survivor = identity_summary(self.db, self.sql, &draft.survivor_id)?;
        self.revision(survivor.revision, draft.survivor_expected_revision)?;
        let mut sources = Vec::with_capacity(draft.sources.len());
        for source in &draft.sources {
            self.target(&source.id, true)?;
            let summary = identity_summary(self.db, self.sql, &source.id)?;
            self.revision(summary.revision, source.expected_revision)?;
            sources.push(summary);
        }
        let affected_relation_ids = affected_relations::<P::Pause>(
            self.db,
            self.sql,
            &draft
                .sources
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
        )?
        .into_iter()
        .map(|(id, _, _)| id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
        let impact = ConceptMergeImpact {
            survivor,
            sources,
            affected_relation_ids,
        };
        self.checked(Input::IdentitySnapshot {
            snapshot_sha256: digest(&impact)?,
        })?;
        Ok(impact)
    }
}
impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(crate) fn write_identity(
        &mut self,
        request: &IdentityRequestV1,
    ) -> Result<String, P::Pause> {
        match request {
            IdentityRequestV1::Merge { draft } => self.merge_identity(draft),
            IdentityRequestV1::Split { draft } => self.split_identity(draft),
            _ => Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
        }
    }
    fn merge_identity(&mut self, draft: &ConceptMergeDraft) -> Result<String, P::Pause> {
        self.checked(Input::IdentityMergeRationale {
            characters: draft.rationale.chars().count(),
        })?;
        let impact = IdentityRead {
            db: self.db,
            sql: self.sql,
            policy: self.policy,
            ordinal: &mut self.ordinal,
        }
        .preview(draft)?;
        let event_id = Uuid::new_v4().to_string();
        let source_ids = impact
            .sources
            .iter()
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        insert_event::<P::Pause>(
            self.db,
            self.sql,
            &event_id,
            "merge",
            &source_ids,
            &impact,
            &draft.rationale,
        )?;
        for source in &source_ids {
            self.db.execute(
                self.sql.statement("concept-identity-mapping-insert.v1.sql"),
                params![source, draft.survivor_id, event_id],
            )?;
        }
        insert_relation_impacts::<P::Pause>(
            self.db,
            self.sql,
            &event_id,
            &source_ids,
            &draft.survivor_id,
            true,
        )?;
        for source in &source_ids {
            self.db.execute(
                self.sql.statement("concept-identity-jobs-cancel.v1.sql"),
                params![source, "concept_merged"],
            )?;
            self.db.execute(
                self.sql
                    .statement("concept-identity-records-tombstone.v1.sql"),
                params![source, "concept_merged"],
            )?;
        }
        self.revise_identity(&draft.survivor_id, "concept_merge")?;
        Ok(event_id)
    }
    fn split_identity(&mut self, draft: &ConceptSplitDraft) -> Result<String, P::Pause> {
        self.checked(split_shape(draft))?;
        let event = identity_event_from::<P::Pause>(self.db, self.sql, &draft.merge_event_id)?;
        self.checked(Input::IdentitySplitEvent {
            snapshot_sha256: digest(&event)?,
            is_merge: event.operation == "merge",
            selected_are_active: draft
                .source_concept_ids
                .iter()
                .all(|id| event.active_source_ids.contains(id)),
        })?;
        let selected = event
            .sources
            .iter()
            .filter(|s| draft.source_concept_ids.contains(&s.id))
            .cloned()
            .collect::<Vec<_>>();
        let affected_relation_ids = affected_relations::<P::Pause>(
            self.db,
            self.sql,
            &draft
                .source_concept_ids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )?
        .into_iter()
        .map(|(id, _, _)| id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
        let impact = ConceptMergeImpact {
            survivor: event.survivor.clone(),
            sources: selected,
            affected_relation_ids,
        };
        self.checked(Input::IdentitySnapshot {
            snapshot_sha256: digest(&impact)?,
        })?;
        let event_id = Uuid::new_v4().to_string();
        insert_event::<P::Pause>(
            self.db,
            self.sql,
            &event_id,
            "split",
            &draft.source_concept_ids,
            &impact,
            &draft.rationale,
        )?;
        for source in &draft.source_concept_ids {
            let changed = self.db.execute(
                self.sql.statement("concept-identity-mapping-delete.v1.sql"),
                params![source, draft.merge_event_id, event.survivor.id],
            )?;
            self.checked(Input::IdentityMappingDeleted { changed })?;
        }
        insert_relation_impacts::<P::Pause>(
            self.db,
            self.sql,
            &event_id,
            &draft.source_concept_ids,
            &event.survivor.id,
            false,
        )?;
        self.revise_identity(&event.survivor.id, "concept_split")?;
        for source in &draft.source_concept_ids {
            self.revise_identity(source, "concept_split")?;
        }
        Ok(event_id)
    }
    fn revise_identity(&mut self, id: &str, reason: &str) -> Result<(), P::Pause> {
        self.db.execute(
            self.sql
                .statement("knowledge-promotion-concept-revision-update.v1.sql"),
            [id],
        )?;
        let concept = self.concept_from(id)?;
        self.save_revision(
            "concept",
            id,
            concept.revision,
            &serde_json::to_value(&concept)
                .map_err(|e| KnowledgeError::InvalidInput(e.to_string()))?,
            "human",
            reason,
        )?;
        self.policy.register_semantic(self.db, &concept)?;
        Ok(())
    }
}
