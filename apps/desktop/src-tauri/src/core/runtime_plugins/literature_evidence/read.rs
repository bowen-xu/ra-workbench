//! Original evidence read order and structural row codecs, with fixed package SQL.
use super::{EvidenceReadError as EvidenceError, EvidenceReadSql};
use research_workbench_native_sdk::evidence::{Claim, EvidenceLink, SourceAnchorSummary};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub(crate) fn list_claims_from(
    sql: &EvidenceReadSql,
    connection: &Connection,
    project_id: Option<&str>,
) -> Result<Vec<Claim>, EvidenceError> {
    let mut statement = connection.prepare(sql.get("claims"))?;
    let rows = statement.query_map(params![project_id], |row| {
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
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn source_from_row(row: &Row<'_>) -> rusqlite::Result<SourceAnchorSummary> {
    let rects_json: String = row.get(9)?;
    Ok(SourceAnchorSummary {
        anchor_id: row.get(0)?,
        annotation_id: row.get(1)?,
        item_id: row.get(2)?,
        item_title: row.get(3)?,
        attachment_id: row.get(4)?,
        attachment_name: row.get(5)?,
        content_sha256: row.get(6)?,
        page_index: row.get(7)?,
        page_label: row.get(8)?,
        rects: serde_json::from_str(&rects_json).unwrap_or_else(|_| serde_json::json!([])),
        quote_exact: row.get(10)?,
        quote_prefix: row.get(11)?,
        quote_suffix: row.get(12)?,
        annotation_type: row.get(13)?,
        annotation_body: row.get(14)?,
        created_at: row.get(15)?,
    })
}

pub(crate) fn source_anchor_from(
    sql: &EvidenceReadSql,
    connection: &Connection,
    anchor_id: &str,
) -> Result<SourceAnchorSummary, EvidenceError> {
    connection
        .query_row(sql.get("source"), [anchor_id], source_from_row)
        .optional()?
        .ok_or(EvidenceError::NotFound)
}

pub(crate) fn list_source_anchors_from(
    sql: &EvidenceReadSql,
    connection: &Connection,
    project_id: Option<&str>,
) -> Result<Vec<SourceAnchorSummary>, EvidenceError> {
    let mut statement = connection.prepare(sql.get("sources"))?;
    let rows = statement.query_map(params![project_id], source_from_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn list_claim_evidence_from(
    sql: &EvidenceReadSql,
    connection: &Connection,
    claim_id: &str,
) -> Result<Vec<EvidenceLink>, EvidenceError> {
    let rows = connection
        .prepare(sql.get("links"))?
        .query_map([claim_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(id, anchor_id, stance, rationale, created_at)| {
            Ok(EvidenceLink {
                id,
                claim_id: claim_id.to_owned(),
                stance,
                rationale,
                source: source_anchor_from(sql, connection, &anchor_id)?,
                created_at,
            })
        })
        .collect()
}
