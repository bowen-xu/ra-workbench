//! Original manual relation preparation before BEGIN and writes on the Core-owned transaction.
use super::validation::checked_decision;
use super::*;
use research_workbench_native_sdk::knowledge_promotion::relation::*;
use rusqlite::Connection;
mod read;
pub(crate) use read::{list_relations, read_relation, search_nodes};

pub(crate) struct PreparedRelation {
    pub(crate) subject: RelationNodeSummary,
    pub(crate) object: RelationNodeSummary,
    pub(crate) current: Option<RelationAssertion>,
}

pub(crate) fn prepare_relation<P: KnowledgePolicy>(
    db: &Connection,
    sql: &KnowledgeSql,
    policy: &mut P,
    draft: &RelationAssertionDraft,
) -> Result<PreparedRelation, P::Pause> {
    checked_decision(policy.decide(
        0,
        Input::RelationDraft {
            facts: Box::new(RelationDraftFactsV1::observe(draft)),
        },
    )?)?;
    let subject = resolve_node(db, sql, policy, 1, &draft.subject)?;
    let object = resolve_node(db, sql, policy, 2, &draft.object)?;
    checked_decision(policy.decide(
        3,
        Input::RelationEndpoints {
            same: subject.kind == object.kind && subject.id == object.id,
            subject_sha256: node_hash(&subject)?,
            object_sha256: node_hash(&object)?,
        },
    )?)?;
    if let Some(project) = &draft.project_id
        && !db.query_row(
            sql.statement("knowledge-promotion-project-exists.v1.sql"),
            [project],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Err(KnowledgeError::NotFound.into());
    }
    if let Some(anchor) = &draft.anchor_id
        && !db.query_row(
            sql.statement("knowledge-promotion-anchor-exists.v1.sql"),
            [anchor],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Err(KnowledgeError::NotFound.into());
    }
    let current = draft
        .id
        .as_deref()
        .map(|id| read_relation(db, sql, id))
        .transpose()?;
    let current_sha256 = current
        .as_ref()
        .map(|value| {
            serde_json::to_vec(value)
                .map(|bytes| crate::runtime_plugins::files::digest(&bytes))
                .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))
        })
        .transpose()?;
    match policy.decide(
        4,
        Input::RelationRevision {
            current_revision: current.as_ref().map(|value| value.revision),
            expected_revision: draft.expected_revision,
            current_sha256,
        },
    )? {
        Decision::RelationRevision { rejection: None } => {}
        Decision::RelationRevision {
            rejection: Some(RelationRevisionRejectionV1::NewExpectedRevision),
        } => {
            return Err(
                KnowledgeError::InvalidInput("新建关系不能指定现有 revision".into()).into(),
            );
        }
        Decision::RelationRevision {
            rejection: Some(RelationRevisionRejectionV1::MissingExpectedRevision),
        } => {
            return Err(
                KnowledgeError::InvalidInput("修改关系必须提供 expectedRevision".into()).into(),
            );
        }
        Decision::RelationRevision {
            rejection: Some(RelationRevisionRejectionV1::Conflict { current_revision }),
        } => return Err(KnowledgeError::RevisionConflict { current_revision }.into()),
        _ => return Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
    }
    Ok(PreparedRelation {
        subject,
        object,
        current,
    })
}
fn node_hash<P>(node: &RelationNodeSummary) -> Result<[u8; 32], P> {
    serde_json::to_vec(node)
        .map(|bytes| crate::runtime_plugins::files::digest(&bytes))
        .map_err(|error| KnowledgeError::InvalidInput(error.to_string()).into())
}
fn resolve_node<P: KnowledgePolicy>(
    db: &Connection,
    sql: &KnowledgeSql,
    policy: &mut P,
    ordinal: usize,
    node: &RelationNodeRef,
) -> Result<RelationNodeSummary, P::Pause> {
    let kind = match policy.decide(
        ordinal,
        Input::RelationNode {
            node_kind: node.kind.chars().take(64).collect(),
            id_is_empty: node.id.trim().is_empty(),
            id_bytes: node.id.len(),
        },
    )? {
        Decision::RelationNode { result } => result.map_err(KnowledgeError::InvalidInput)?,
        _ => return Err(PromotionError::Failed(AiError::TemporaryUnavailable)),
    };
    db.query_row(
        sql.statement(&format!("relation-node-read-{}.v1.sql", kind.as_str())),
        [&node.id],
        |row| read::node_from_row(row, kind),
    )
    .optional()?
    .ok_or_else(|| KnowledgeError::NotFound.into())
}

impl<P: KnowledgePolicy> KnowledgeIo<'_, '_, P> {
    pub(crate) fn write_relation(
        &mut self,
        draft: &RelationAssertionDraft,
        prepared: &PreparedRelation,
    ) -> Result<String, P::Pause> {
        let id = draft
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let current = prepared.current.as_ref();
        let revision = current.map_or(1, |value| value.revision + 1);
        let subject = &prepared.subject;
        let object = &prepared.object;
        let scope_note = draft.scope_note.trim();
        let arguments = params![
            id,
            draft.project_id,
            subject.kind,
            subject.id,
            subject.label,
            draft.predicate_kind,
            object.kind,
            object.id,
            object.label,
            scope_note
        ];
        if let Some(existing) = current {
            let mut bindings = arguments.to_vec();
            bindings.push(&revision);
            bindings.push(&existing.revision);
            let updated = self.db.execute(
                self.sql.statement("relation-manual-update.v1.sql"),
                bindings.as_slice(),
            )?;
            if updated != 1 {
                return Err(KnowledgeError::RevisionConflict {
                    current_revision: read_relation::<P::Pause>(self.db, self.sql, &id)?.revision,
                }
                .into());
            }
        } else {
            self.db.execute(
                self.sql.statement("relation-manual-insert.v1.sql"),
                arguments,
            )?;
        }
        let provenance = current.map_or("human_original", |value| value.provenance_kind.as_str());
        let snapshot = serde_json::json!({
            "id": id,
            "projectId": draft.project_id,
            "subject": {"kind": subject.kind, "id": subject.id, "label": subject.label},
            "predicateKind": draft.predicate_kind,
            "object": {"kind": object.kind, "id": object.id, "label": object.label},
            "scopeNote": scope_note,
            "provenanceKind": provenance,
            "epistemicRole": "assertion",
            "revision": revision,
        });
        let reason = draft
            .change_reason
            .as_deref()
            .unwrap_or(if current.is_some() {
                "manual_edit"
            } else {
                "manual_create"
            });
        self.save_revision(
            "relation_assertion",
            &id,
            revision,
            &snapshot,
            "human",
            reason,
        )?;
        if let (Some(anchor), Some(stance)) = (&draft.anchor_id, &draft.evidence_stance) {
            self.db.execute(
                self.sql.statement("relation-manual-evidence-insert.v1.sql"),
                params![
                    Uuid::new_v4().to_string(),
                    id,
                    anchor,
                    stance,
                    draft.evidence_rationale.as_deref().unwrap_or("").trim()
                ],
            )?;
        }
        Ok(id)
    }
}
