//! Original fixed claim and link storage, including original readback order.
use super::{ClaimsError as EvidenceError, ClaimsPolicy, ClaimsSql};
use research_workbench_native_sdk::evidence_write::*;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;
#[derive(Default)]
pub(crate) struct LinkProgress {
    pub(crate) attempted: bool,
    pub(crate) returned: bool,
    pub(crate) id: Option<String>,
}

pub(crate) fn claim_from(
    sql: &ClaimsSql,
    connection: &Connection,
    id: &str,
) -> Result<Claim, EvidenceError> {
    connection
        .query_row(sql.get("claim"), [id], |row| {
            Ok(Claim {
                id: row.get(0)?,
                project_id: row.get(1)?,
                project_title: row.get(2)?,
                statement: row.get(3)?,
                origin: row.get(4)?,
                provenance_kind: row.get(5)?,
                epistemic_role: row.get(6)?,
                revision: row.get(7)?,
                evidence_count: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .optional()?
        .ok_or(EvidenceError::NotFound)
}

pub(crate) fn save_claim_in(
    sql: &ClaimsSql,
    policy: &mut dyn ClaimsPolicy,
    connection: &Connection,
    mut draft: ClaimDraft,
    identity: &mut Option<String>,
) -> Result<Claim, EvidenceError> {
    draft.statement = draft.statement.trim().to_owned();
    policy.validate(EvidenceWriteInputV1::Claim {
        statement: draft.statement.clone(),
        epistemic_role: draft.epistemic_role.clone(),
    })?;
    let id = draft.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    *identity = Some(id.clone());
    let existing = connection
        .query_row(sql.get("revision"), [&id], |row| row.get::<_, i64>(0))
        .optional()?;
    let revision = if let Some(current_revision) = existing {
        if draft.expected_revision != Some(current_revision) {
            return Err(EvidenceError::RevisionConflict { current_revision });
        }
        connection.execute(
            sql.get("update"),
            params![
                id,
                draft.project_id,
                draft.statement,
                draft.epistemic_role,
                current_revision + 1
            ],
        )?;
        current_revision + 1
    } else {
        if draft.expected_revision.is_some() {
            return Err(EvidenceError::NotFound);
        }
        connection.execute(
            sql.get("insert"),
            params![id, draft.project_id, draft.statement, draft.epistemic_role],
        )?;
        1
    };
    let claim = claim_from(sql, connection, &id)?;
    connection.execute(
        sql.get("history"),
        params![
            Uuid::new_v4().to_string(),
            id,
            revision,
            serde_json::to_string(&serde_json::json!({
                "id": claim.id,
                "projectId": claim.project_id,
                "statement": claim.statement,
                "provenanceKind": claim.provenance_kind,
                "epistemicRole": claim.epistemic_role,
                "revision": claim.revision
            }))?,
            draft.change_reason.as_deref().unwrap_or("manual_save")
        ],
    )?;
    claim_from(sql, connection, &id)
}

pub(crate) fn link_evidence_in(
    sql: &ClaimsSql,
    policy: &mut dyn ClaimsPolicy,
    progress: &mut LinkProgress,
    connection: &Connection,
    mut draft: EvidenceLinkDraft,
) -> Result<EvidenceLink, EvidenceError> {
    policy.validate(EvidenceWriteInputV1::Stance {
        stance: draft.stance.clone(),
    })?;
    draft.rationale = draft.rationale.trim().to_owned();
    let id = Uuid::new_v4().to_string();
    progress.attempted = true;
    connection.execute(
        sql.get("link-upsert"),
        params![
            id,
            draft.claim_id,
            draft.anchor_id,
            draft.stance,
            draft.rationale
        ],
    )?;
    progress.returned = true;
    let (stored_id, created_at): (String, String) = connection
        .query_row(
            sql.get("link-lookup"),
            params![draft.claim_id, draft.anchor_id, draft.stance],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(EvidenceError::NotFound)?;
    progress.id = Some(stored_id.clone());
    Ok(EvidenceLink {
        id: stored_id,
        claim_id: draft.claim_id,
        stance: draft.stance,
        rationale: draft.rationale,
        source: super::super::literature_evidence::source_anchor_from(
            &sql.reads,
            connection,
            &draft.anchor_id,
        )
        .map_err(EvidenceError::from)?,
        created_at,
    })
}
