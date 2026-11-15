//! Original fixed node/relation row codecs. All database access remains in Core.
use super::*;
pub(super) fn node_from_row(
    row: &rusqlite::Row<'_>,
    kind: RelationNodeKindV1,
) -> rusqlite::Result<RelationNodeSummary> {
    Ok(RelationNodeSummary {
        kind: kind.as_str().into(),
        id: row.get(0)?,
        label: row.get(1)?,
        detail: row.get(2)?,
    })
}
pub(crate) fn read_relation<P>(
    db: &Connection,
    sql: &KnowledgeSql,
    id: &str,
) -> Result<RelationAssertion, P> {
    db.query_row(sql.statement("relation-manual-read.v1.sql"), [id], |row| {
        Ok(RelationAssertion {
            id: row.get(0)?,
            project_id: row.get(1)?,
            project_title: row.get(2)?,
            subject: RelationNodeSummary {
                kind: row.get(3)?,
                id: row.get(4)?,
                label: row.get(5)?,
                detail: String::new(),
            },
            predicate_kind: row.get(6)?,
            object: RelationNodeSummary {
                kind: row.get(7)?,
                id: row.get(8)?,
                label: row.get(9)?,
                detail: String::new(),
            },
            scope_note: row.get(10)?,
            provenance_kind: row.get(11)?,
            epistemic_role: row.get(12)?,
            revision: row.get(13)?,
            evidence_count: row.get(14)?,
            created_at: row.get(15)?,
            updated_at: row.get(16)?,
        })
    })
    .optional()?
    .ok_or_else(|| KnowledgeError::NotFound.into())
}
pub(crate) fn list_relations<P>(
    db: &Connection,
    sql: &KnowledgeSql,
    project_id: Option<&str>,
) -> Result<Vec<RelationAssertion>, P> {
    let ids = db
        .prepare(sql.statement("relation-manual-list.v1.sql"))?
        .query_map([project_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ids.iter().map(|id| read_relation(db, sql, id)).collect()
}
pub(crate) fn search_nodes<P>(
    db: &Connection,
    sql: &KnowledgeSql,
    query: &str,
    plan: &RelationSearchPlanV1,
) -> Result<Vec<RelationNodeSummary>, P> {
    let mut nodes = Vec::new();
    for kind in plan.kinds {
        nodes.extend(
            db.prepare(sql.statement(&format!("relation-node-search-{}.v1.sql", kind.as_str())))?
                .query_map([query.trim()], |row| node_from_row(row, kind))?
                .collect::<std::result::Result<Vec<_>, _>>()?,
        );
    }
    nodes.truncate(plan.final_limit);
    Ok(nodes)
}
