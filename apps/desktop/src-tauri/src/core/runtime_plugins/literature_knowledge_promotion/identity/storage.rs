//! Original fixed identity reads and ordered event/impact writes.
use super::*;
pub(crate) fn list_events<P>(
    connection: &Connection,
    sql: &KnowledgeSql,
) -> Result<Vec<ConceptIdentityEvent>, P> {
    let ids = connection
        .prepare(sql.statement("concept-identity-events-list.v1.sql"))?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ids.iter()
        .map(|id| identity_event_from(connection, sql, id))
        .collect()
}

pub(super) fn identity_summary<P>(
    connection: &Connection,
    sql: &KnowledgeSql,
    concept_id: &str,
) -> Result<ConceptIdentitySummary, P> {
    connection
        .query_row(
            sql.statement("concept-identity-summary.v1.sql"),
            [concept_id],
            |row| {
                Ok(ConceptIdentitySummary {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    revision: row.get(2)?,
                    term_count: row.get(3)?,
                    definition_count: row.get(4)?,
                    source_count: row.get(5)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| PromotionError::from(KnowledgeError::NotFound))
}

pub(super) fn affected_relations<P>(
    connection: &Connection,
    sql: &KnowledgeSql,
    source_ids: &[&str],
) -> Result<Vec<(String, String, String)>, P> {
    let mut impacts = Vec::new();
    for source_id in source_ids {
        let mut statement =
            connection.prepare(sql.statement("concept-identity-relations.v1.sql"))?;
        impacts.extend(
            statement
                .query_map([source_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    Ok(impacts)
}

pub(super) fn insert_event<P>(
    connection: &Connection,
    sql: &KnowledgeSql,
    event_id: &str,
    operation: &str,
    source_ids: &[String],
    impact: &ConceptMergeImpact,
    rationale: &str,
) -> Result<(), P> {
    connection.execute(
        sql.statement("concept-identity-event-insert.v1.sql"),
        params![
            event_id,
            operation,
            impact.survivor.id,
            serde_json::to_string(source_ids)
                .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?,
            serde_json::to_string(impact)
                .map_err(|error| KnowledgeError::InvalidInput(error.to_string()))?,
            rationale.trim(),
        ],
    )?;
    Ok(())
}

pub(super) fn insert_relation_impacts<P>(
    connection: &Connection,
    sql: &KnowledgeSql,
    event_id: &str,
    source_ids: &[String],
    survivor_id: &str,
    is_merge: bool,
) -> Result<(), P> {
    for (relation_id, role, source_id) in affected_relations(
        connection,
        sql,
        &source_ids.iter().map(String::as_str).collect::<Vec<_>>(),
    )? {
        let (original, resolved) = if is_merge {
            (source_id.as_str(), survivor_id)
        } else {
            (survivor_id, source_id.as_str())
        };
        connection.execute(
            sql.statement("concept-identity-impact-insert.v1.sql"),
            params![event_id, relation_id, role, original, resolved],
        )?;
    }
    Ok(())
}

pub(crate) fn identity_event_from<P>(
    connection: &Connection,
    sql: &KnowledgeSql,
    event_id: &str,
) -> Result<ConceptIdentityEvent, P> {
    let (operation, snapshot_json, rationale, created_at) = connection
        .query_row(
            sql.statement("concept-identity-event-read.v1.sql"),
            [event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| PromotionError::from(KnowledgeError::NotFound))?;
    let impact: ConceptMergeImpact = serde_json::from_str(&snapshot_json)
        .map_err(|_| KnowledgeError::InvalidInput("概念身份事件快照损坏".to_owned()))?;
    let active_source_ids = connection
        .prepare(sql.statement("concept-identity-active-sources.v1.sql"))?
        .query_map([event_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let affected_relation_ids = connection
        .prepare(sql.statement("concept-identity-event-relations.v1.sql"))?
        .query_map([event_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ConceptIdentityEvent {
        id: event_id.to_owned(),
        operation,
        survivor: impact.survivor,
        sources: impact.sources,
        affected_relation_ids,
        active_source_ids,
        rationale,
        created_at,
    })
}
