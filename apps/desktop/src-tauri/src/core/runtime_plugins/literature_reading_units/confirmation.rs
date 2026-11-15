//! Original confirmed-unit storage. Pure validation is supplied by the selected literature rules.
mod anchors;
mod resources;
use crate::ai::{
    error::AiError, knowledge_contracts::KnowledgeError, structured_resource_validation::Flow,
};
pub(crate) use anchors::ConfirmationAnchorSql;
use research_workbench_native_sdk::paper_confirmation::unit::*;
pub(crate) use resources::ConfirmationUnitSql;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

pub(crate) struct AiConfirmedReadingUnitDraft<'a> {
    pub template_field: Option<(&'a str, &'a str)>,
    pub project_id: Option<&'a str>,
    pub paper_id: &'a str,
    pub unit_kind: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub provenance_kind: &'a str,
    pub epistemic_role: &'a str,
    pub anchor_ids: &'a [String],
    pub candidate_id: &'a str,
    pub candidate_revision: i64,
}

pub(crate) struct AiReadingUnitConfirmation {
    pub reading_unit_id: String,
    pub applied_revision: i64,
    pub previous_snapshot: Option<serde_json::Value>,
}

pub(crate) struct AiReadingUnitUndoDraft<'a> {
    pub reading_unit_id: &'a str,
    pub expected_revision: i64,
    pub previous_snapshot: Option<&'a serde_json::Value>,
    pub confirmation_batch_id: &'a str,
}

fn problem(error: UnitProblemV1) -> KnowledgeError {
    match error {
        UnitProblemV1::InvalidInput { message } => KnowledgeError::InvalidInput(message),
        UnitProblemV1::NotFound {} => KnowledgeError::NotFound,
        UnitProblemV1::RevisionConflict { current_revision } => {
            KnowledgeError::RevisionConflict { current_revision }
        }
    }
}
fn mismatch() -> KnowledgeError {
    KnowledgeError::InvalidInput("字段确认阅读单元结果种类不匹配".into())
}

pub(crate) fn confirm<P>(
    sql: &ConfirmationUnitSql,
    transaction: &rusqlite::Transaction<'_>,
    draft: AiConfirmedReadingUnitDraft<'_>,
    policy: &mut impl FnMut(UnitInputV1) -> Result<UnitDecisionV1, Flow<AiError, P>>,
) -> Result<AiReadingUnitConfirmation, Flow<AiError, P>> {
    let decision = policy(UnitInputV1::Validate {
        title: draft.title.into(),
        body: draft.body.into(),
        unit_kind: draft.unit_kind.into(),
        provenance_kind: draft.provenance_kind.into(),
        epistemic_role: draft.epistemic_role.into(),
        anchor_count: draft.anchor_ids.len(),
    })?;
    let UnitDecisionV1::Validated { rejection } = decision else {
        return Err(mismatch().into());
    };
    if let Some(error) = rejection {
        return Err(problem(error).into());
    }
    let existing_id = if let Some((portrait, field)) = draft.template_field {
        transaction
            .query_row(
                sql.template_lookup.trim_end_matches('\n'),
                params![draft.paper_id, portrait, field],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(KnowledgeError::from)?
    } else {
        transaction
            .query_row(
                sql.ordinary_lookup.trim_end_matches('\n'),
                params![draft.paper_id, draft.unit_kind, draft.project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(KnowledgeError::from)?
    };
    let Some(id) = existing_id else {
        let accepted =
            research_workbench_native_sdk::reading_units::accepted::AcceptedReadingUnitV1 {
                project_id: draft.project_id.map(str::to_owned),
                paper_id: draft.paper_id.into(),
                unit_kind: draft.unit_kind.into(),
                title: draft.title.into(),
                body: draft.body.into(),
                provenance_kind: draft.provenance_kind.into(),
                epistemic_role: draft.epistemic_role.into(),
                anchor_ids: draft.anchor_ids.to_vec(),
                candidate_id: draft.candidate_id.into(),
                candidate_revision: draft.candidate_revision,
            };
        // The original combined confirmation check already proves both acceptance checks.
        let (id, applied_revision) = sql
            .acceptance
            .apply(transaction, &accepted, &mut |_| {
                Ok::<(), crate::ai::error::AiError>(())
            })
            .map_err(|error| match error {
                crate::ai::error::AiError::Database(error) => KnowledgeError::Database(error),
                _ => unreachable!("qualified acceptance only executes fixed SQL"),
            })?;
        if let Some((portrait, field)) = draft.template_field {
            transaction
                .execute(
                    sql.template_link.trim_end_matches('\n'),
                    params![id, portrait, field],
                )
                .map_err(KnowledgeError::from)?;
        }
        return Ok(AiReadingUnitConfirmation {
            reading_unit_id: id,
            applied_revision,
            previous_snapshot: None,
        });
    };

    let previous_snapshot = reading_unit_snapshot(sql, transaction, &id)?;
    let current_revision = previous_snapshot["revision"]
        .as_i64()
        .ok_or_else(|| KnowledgeError::InvalidInput("阅读单元历史快照无效".to_owned()))?;
    let next_revision = current_revision + 1;
    transaction
        .execute(
            sql.update.trim_end_matches('\n'),
            params![
                id,
                draft.project_id,
                draft.paper_id,
                draft.unit_kind,
                draft.title.trim(),
                draft.body.trim(),
                draft.provenance_kind,
                draft.epistemic_role,
                next_revision,
                current_revision,
            ],
        )
        .map_err(KnowledgeError::from)?;
    replace_evidence_links(
        sql,
        transaction,
        &id,
        draft.anchor_ids,
        &format!(
            "AI candidate {} revision {} field confirmed by human",
            draft.candidate_id, draft.candidate_revision
        ),
    )?;
    let snapshot = serde_json::json!({
        "id": id,
        "projectId": draft.project_id,
        "paperId": draft.paper_id,
        "unitKind": draft.unit_kind,
        "title": draft.title.trim(),
        "body": draft.body.trim(),
        "provenanceKind": draft.provenance_kind,
        "epistemicRole": draft.epistemic_role,
        "revision": next_revision,
        "acceptedFrom": {
            "candidateId": draft.candidate_id,
            "candidateRevision": draft.candidate_revision
        }
    });
    save_revision(
        sql,
        transaction,
        "reading_unit",
        &id,
        next_revision,
        &snapshot,
        "ai_review",
        "accepted_ai_candidate_field",
    )?;
    Ok(AiReadingUnitConfirmation {
        reading_unit_id: id,
        applied_revision: next_revision,
        previous_snapshot: Some(previous_snapshot),
    })
}

pub(crate) fn undo<P>(
    sql: &ConfirmationUnitSql,
    transaction: &rusqlite::Transaction<'_>,
    draft: AiReadingUnitUndoDraft<'_>,
    policy: &mut impl FnMut(UnitInputV1) -> Result<UnitDecisionV1, Flow<AiError, P>>,
) -> Result<i64, Flow<AiError, P>> {
    let current_revision = transaction
        .query_row(
            sql.current_revision.trim_end_matches('\n'),
            [draft.reading_unit_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(KnowledgeError::from)?;
    let decision = policy(UnitInputV1::Revision {
        previous_snapshot: draft.previous_snapshot.cloned(),
        current_revision,
        expected_revision: draft.expected_revision,
    })?;
    let UnitDecisionV1::Revision { result } = decision else {
        return Err(mismatch().into());
    };
    let next_revision = result.map_err(problem)?;
    let current_revision = draft.expected_revision;
    match draft.previous_snapshot {
        None => {
            transaction
                .execute(
                    sql.revert.trim_end_matches('\n'),
                    params![draft.reading_unit_id, next_revision, current_revision],
                )
                .map_err(KnowledgeError::from)?;
            save_revision(
                sql,
                transaction,
                "reading_unit",
                draft.reading_unit_id,
                next_revision,
                &serde_json::json!({
                    "id": draft.reading_unit_id,
                    "revision": next_revision,
                    "reverted": true,
                    "confirmationBatchId": draft.confirmation_batch_id
                }),
                "human",
                "undo_ai_confirmation_created_unit",
            )?;
        }
        Some(snapshot) => {
            let decision = policy(UnitInputV1::Restore {
                snapshot: snapshot.clone(),
            })?;
            let UnitDecisionV1::Restore { header, links } = decision else {
                return Err(mismatch().into());
            };
            let restored_unit = header.map_err(problem)?;
            let RestoredUnitV1 {
                project_id,
                paper_id,
                unit_kind,
                title,
                body,
                provenance_kind,
                epistemic_role,
            } = restored_unit;
            transaction
                .execute(
                    sql.restore.trim_end_matches('\n'),
                    params![
                        draft.reading_unit_id,
                        project_id,
                        paper_id,
                        unit_kind,
                        title,
                        body,
                        provenance_kind,
                        epistemic_role,
                        next_revision,
                        current_revision,
                    ],
                )
                .map_err(KnowledgeError::from)?;
            restore_evidence_links(sql, transaction, draft.reading_unit_id, links)?;
            let mut restored = snapshot.clone();
            restored["revision"] = serde_json::json!(next_revision);
            restored["restoredFromConfirmationBatchId"] =
                serde_json::json!(draft.confirmation_batch_id);
            save_revision(
                sql,
                transaction,
                "reading_unit",
                draft.reading_unit_id,
                next_revision,
                &restored,
                "human",
                "undo_ai_confirmation_restore_revision",
            )?;
        }
    }
    Ok(next_revision)
}

fn reading_unit_snapshot(
    sql: &ConfirmationUnitSql,
    connection: &Connection,
    id: &str,
) -> Result<serde_json::Value, KnowledgeError> {
    let mut snapshot = connection
        .query_row(sql.snapshot.trim_end_matches('\n'), [id], |row| {
            Ok(serde_json::json!({
                "id": id,
                "projectId": row.get::<_, Option<String>>(0)?,
                "paperId": row.get::<_, Option<String>>(1)?,
                "unitKind": row.get::<_, String>(2)?,
                "title": row.get::<_, String>(3)?,
                "body": row.get::<_, String>(4)?,
                "provenanceKind": row.get::<_, String>(5)?,
                "epistemicRole": row.get::<_, String>(6)?,
                "revision": row.get::<_, i64>(7)?,
                "evidenceLinks": []
            }))
        })
        .optional()?
        .ok_or(KnowledgeError::NotFound)?;
    let links = connection
        .prepare(sql.snapshot_evidence.trim_end_matches('\n'))?
        .query_map([id], |row| {
            Ok(serde_json::json!({
                "anchorId": row.get::<_, String>(0)?,
                "stance": row.get::<_, String>(1)?,
                "rationale": row.get::<_, String>(2)?,
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    snapshot["evidenceLinks"] = serde_json::json!(links);
    Ok(snapshot)
}

fn replace_evidence_links(
    sql: &ConfirmationUnitSql,
    transaction: &rusqlite::Transaction<'_>,
    reading_unit_id: &str,
    anchor_ids: &[String],
    rationale: &str,
) -> Result<(), KnowledgeError> {
    transaction.execute(
        sql.evidence_delete.trim_end_matches('\n'),
        [reading_unit_id],
    )?;
    for anchor_id in anchor_ids {
        transaction.execute(
            sql.evidence_insert.trim_end_matches('\n'),
            params![
                Uuid::new_v4().to_string(),
                reading_unit_id,
                anchor_id,
                rationale
            ],
        )?;
    }
    Ok(())
}

fn restore_evidence_links(
    sql: &ConfirmationUnitSql,
    transaction: &rusqlite::Transaction<'_>,
    reading_unit_id: &str,
    links: Result<Vec<Result<RestoredEvidenceV1, UnitProblemV1>>, UnitProblemV1>,
) -> Result<(), KnowledgeError> {
    transaction.execute(
        sql.evidence_delete.trim_end_matches('\n'),
        [reading_unit_id],
    )?;
    let links = links.map_err(problem)?;
    for link in links {
        let link = link.map_err(problem)?;
        transaction.execute(
            sql.evidence_restore.trim_end_matches('\n'),
            params![
                Uuid::new_v4().to_string(),
                reading_unit_id,
                link.anchor_id,
                link.stance,
                link.rationale,
            ],
        )?;
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "original fixed revision bindings plus accepted SQL resources"
)]
fn save_revision(
    sql: &ConfirmationUnitSql,
    connection: &Connection,
    object_kind: &str,
    object_id: &str,
    revision: i64,
    snapshot: &serde_json::Value,
    editor_kind: &str,
    change_reason: &str,
) -> Result<(), KnowledgeError> {
    connection.execute(
        sql.revision_insert.trim_end_matches('\n'),
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
