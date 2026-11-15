//! Original SQL consumption and row-decode order in one caller-owned read tx.
use super::resources::QuerySql;
use research_workbench_native_sdk::{
    data::{DataRows, DataValue},
    writing_catalog_query::{QueryCursor, QueryPlan, QueryRead, QuerySnapshot},
};
use rusqlite::{Connection, OptionalExtension, Statement, types::Value};

fn prepare<'a>(db: &'a Connection, sql: &str) -> rusqlite::Result<Statement<'a>> {
    let statement = db.prepare(sql)?;
    if !statement.readonly() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(statement)
}
fn scoped<'a>(
    db: &'a Connection,
    sql: &QuerySql,
    kind: &str,
    plan: &QueryPlan,
) -> rusqlite::Result<Statement<'a>> {
    let searching = !plan.terms.is_empty();
    let mut statement = prepare(db, sql.scoped(kind, searching))?;
    if !searching {
        statement.raw_bind_parameter(1, &plan.status)?;
    }
    Ok(statement)
}
fn strings(row: &rusqlite::Row<'_>, count: usize) -> rusqlite::Result<Vec<DataValue>> {
    (0..count)
        .map(|index| row.get::<_, String>(index).map(DataValue::Text))
        .collect()
}
fn rows(
    statement: &mut Statement<'_>,
    decode: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<Vec<DataValue>>,
) -> rusqlite::Result<DataRows> {
    let mut rows = statement.raw_query();
    let mut result = Vec::new();
    while let Some(row) = rows.next()? {
        result.push(decode(row)?);
    }
    Ok(DataRows { rows: result })
}
pub(crate) fn capture(
    db: &Connection,
    sql: &QuerySql,
    plan: &QueryPlan,
) -> rusqlite::Result<QuerySnapshot> {
    let revision = prepare(db, sql.get("catalog-query-generation.v1.sql"))?
        .query_row([], |row| row.get::<_, i64>(0))?;
    let snapshot = |read| QuerySnapshot { revision, read };
    match &plan.cursor {
        QueryCursor::Rejected { .. } => return Ok(snapshot(QueryRead::CursorBoundary {})),
        QueryCursor::Ready { cursor } if cursor.revision != revision => {
            return Ok(snapshot(QueryRead::CursorBoundary {}));
        }
        _ => {}
    }
    if !plan.includes {
        return Ok(snapshot(QueryRead::Excluded {}));
    }
    let versions = prepare(db, sql.get("catalog-query-versions.v1.sql"))?.query_row([], |row| {
        Ok([row.get::<_, String>(0)?, row.get(1)?, row.get(2)?])
    })?;
    if versions != plan.versions {
        return Ok(snapshot(QueryRead::VersionMismatch { versions }));
    }
    let mut timestamp = scoped(db, sql, "timestamp", plan)?;
    let mut timestamp_rows = timestamp.raw_query();
    if let Some(row) = timestamp_rows.next()? {
        return Ok(snapshot(QueryRead::TimestampInvalid {
            manuscript_id: row.get(0)?,
            path: row.get(1)?,
        }));
    }
    drop(timestamp_rows);
    drop(timestamp);
    let counts = rows(&mut scoped(db, sql, "counts", plan)?, |row| {
        Ok(vec![
            DataValue::Text(row.get(0)?),
            DataValue::Integer(row.get(1)?),
        ])
    })?;
    let issues = rows(&mut scoped(db, sql, "issues", plan)?, |row| strings(row, 3))?;
    let (after, cursor_id) = match &plan.cursor {
        QueryCursor::Ready { cursor } => {
            let mut statement = prepare(
                db,
                sql.get(&format!(
                    "catalog-query-after-{}.v1.sql",
                    plan.order.asset_key()
                )),
            )?;
            match statement
                .query_row([&cursor.id], |row| row.get::<_, Value>(0))
                .optional()?
            {
                Some(value) => (value, cursor.id.as_str()),
                None => return Ok(snapshot(QueryRead::CursorMissing {})),
            }
        }
        _ => (Value::Null, ""),
    };
    let mut statement = prepare(db, &sql.items(plan))?;
    statement.raw_bind_parameter(1, &plan.status)?;
    statement.raw_bind_parameter(2, after)?;
    statement.raw_bind_parameter(3, cursor_id)?;
    statement.raw_bind_parameter(4, (plan.limit + 1) as i64)?;
    for (index, term) in plan.terms.iter().enumerate() {
        statement.raw_bind_parameter(index + 5, &term.parameter)?;
        statement.raw_bind_parameter(index + 5 + plan.terms.len(), &term.raw)?;
    }
    let searching = !plan.terms.is_empty();
    let items = rows(&mut statement, |row| {
        // Original snippet-before-item, then manuscript identity-before-metadata.
        let snippet = if searching {
            row.get(7)?
        } else {
            String::new()
        };
        let manuscript: String = row.get(5)?;
        let mut values = vec![DataValue::Null; 8];
        values[5] = DataValue::Text(manuscript);
        for index in [0, 1, 2, 3, 4, 6] {
            values[index] = DataValue::Text(row.get(index)?);
        }
        values[7] = DataValue::Text(snippet);
        Ok(values)
    })?;
    Ok(snapshot(QueryRead::Ready {
        counts,
        issues,
        items,
    }))
}
