//! Existing search IPC over the shared derived text index. Source filters run
//! before LIMIT; source metadata is hydrated only for returned reading hits.
use super::{NotesError, invalid, search};
use crate::notes::{
    model::{NoteSearchPage, NoteSearchQuery, NoteSearchResult},
    reading_note_repository::reading_note_metadata_from,
};
use rusqlite::{Connection, types::Value};

fn sql(query: &NoteSearchQuery, terms: &[String], values: &mut Vec<Value>) -> String {
    let kinds = query.object_kinds.as_deref();
    let include_reading = kinds.is_none_or(|kinds| kinds.iter().any(|kind| kind == "reading_note"));
    let mut filter = String::from("c.status=?1");
    for (value, condition) in [
        (
            query.project_id.as_ref(),
            "exists(select 1 from project_note_pages x where x.page_id=c.id and x.project_id=?)",
        ),
        (
            query.tag_id.as_ref(),
            "exists(select 1 from note_page_tags x where x.page_id=c.id and x.tag_id=?)",
        ),
    ] {
        if let Some(value) = value {
            values.push(Value::Text(value.clone()));
            filter.push_str(&format!(
                " and {}",
                condition.replace('?', &format!("?{}", values.len()))
            ));
        }
    }
    if let Some(favorite) = query.is_favorite {
        values.push(Value::Integer(i64::from(favorite)));
        filter.push_str(&format!(" and n.is_favorite=?{}", values.len()));
    }
    let mut reading_filter = filter.clone();
    for (value, column) in [
        (query.paper_id.as_ref(), "s.paper_id_snapshot"),
        (query.note_type.as_ref(), "s.note_type"),
        (
            query.source_state.as_ref(),
            "case when p.id is null then 'unavailable' when p.trashed_at is not null then 'paper_in_trash' when s.current_anchor_id is null then 'page_only' when a.content_sha256<>s.content_sha256 then 'needs_relocation' when s.note_type='page_note' then 'page_only' else 'exact' end",
        ),
    ] {
        if let Some(value) = value.filter(|_| include_reading) {
            values.push(Value::Text(value.clone()));
            reading_filter.push_str(&format!(" and ({column})=?{}", values.len()));
        }
    }
    // A sorted ID range otherwise tempts SQLite to visit the entire header
    // range before testing a rare FTS hit. Keep matches as the outer loop:
    // https://sqlite.org/optoverview.html#manual_control_of_query_plans_using_cross_join
    let (ordinary_from, reading_from) = if terms.is_empty() {
        (
            "note_page_catalog c join note_pages n on n.id=c.id",
            "reading_note_sources s join note_page_catalog c on c.id=s.page_id join note_pages n on n.id=c.id",
        )
    } else {
        (
            "matches m cross join note_page_catalog c on c.id=m.page_id cross join note_pages n on n.id=c.id",
            "matches m cross join reading_note_sources s on s.page_id=m.page_id cross join note_page_catalog c on c.id=s.page_id cross join note_pages n on n.id=c.id",
        )
    };
    let preview_join = if terms.is_empty() {
        String::new()
    } else {
        " left join note_catalog_blocks b on b.block_id=m.preview_block".to_owned()
    };
    let snippet = if terms.is_empty() {
        "c.title"
    } else {
        "coalesce(substr(b.search_text,1,240),c.title)"
    };
    let ordinary = format!("select c.id as object_id,'note_page' as object_kind,c.title,{snippet} as snippet,c.updated_at
        from {ordinary_from}{preview_join} where c.source_id is null and c.id>=?2 and (c.id>?2 or 'note_page'>?3) and {filter}");
    let reading = format!("select s.block_id as object_id,'reading_note' as object_kind,s.paper_title_snapshot as title,{snippet} as snippet,c.updated_at
        from {reading_from}
        left join bibliographic_items p on p.id=s.paper_id_snapshot left join attachments a on a.id=s.attachment_id_snapshot
        {preview_join} where s.block_id>=?2 and (s.block_id>?2 or 'reading_note'>?3) and {reading_filter}");
    let mut branches = Vec::new();
    if kinds.is_none_or(|kinds| kinds.iter().any(|kind| kind == "note_page")) {
        branches.push(ordinary);
    }
    if include_reading {
        branches.push(reading);
    }
    format!(
        "{} {} order by object_id,object_kind limit ?4",
        if terms.is_empty() {
            String::new()
        } else {
            search::matching_pages_sql(terms)
        },
        branches.join(" union all ")
    )
}

pub(crate) fn search_from(
    connection: &Connection,
    query: NoteSearchQuery,
) -> Result<NoteSearchPage, NotesError> {
    if query.object_kinds.as_ref().is_some_and(|kinds| {
        kinds
            .iter()
            .any(|kind| !matches!(kind.as_str(), "note_page" | "reading_note"))
    }) {
        return Err(invalid("搜索对象类型无效"));
    }
    let status = query.status.as_deref().unwrap_or("active");
    if !crate::notes::model::validate_status(status) {
        return Err(invalid("搜索状态无效"));
    }
    let terms = search::terms(query.search.as_deref().unwrap_or_default())?;
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let (id, kind): (String, String) = match query.cursor.as_deref() {
        None | Some("") => (String::new(), String::new()),
        Some(value) if value.len() <= 8192 => {
            serde_json::from_str(value).map_err(|_| invalid("搜索游标无效"))?
        }
        _ => return Err(invalid("搜索游标无效")),
    };
    if query.object_kinds.as_ref().is_some_and(Vec::is_empty) {
        return Ok(NoteSearchPage {
            items: Vec::new(),
            next_cursor: None,
            index_state: "ready".to_owned(),
        });
    }
    let tx = if terms.is_empty() {
        connection.unchecked_transaction()?
    } else {
        super::metadata_projection::snapshot(connection, status)?
    };
    let mut values = vec![
        Value::Text(status.to_owned()),
        Value::Text(id),
        Value::Text(kind),
        Value::Integer((limit + 1) as i64),
    ];
    values.extend(
        terms
            .iter()
            .map(|term| Value::Text(format!("\"{}\"", term.replace('"', "\"\"")))),
    );
    values.extend(terms.iter().cloned().map(Value::Text));
    let statement_sql = sql(&query, &terms, &mut values);
    let mut items = {
        let mut statement = tx.prepare(&statement_sql)?;
        statement
            .query_map(rusqlite::params_from_iter(values), |row| {
                Ok(NoteSearchResult {
                    object_id: row.get(0)?,
                    object_kind: row.get(1)?,
                    title: row.get(2)?,
                    snippet: row.get(3)?,
                    updated_at: row.get(4)?,
                    source: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let more = items.len() > limit;
    items.truncate(limit);
    for item in &mut items {
        if item.object_kind == "reading_note" {
            item.source = Some(reading_note_metadata_from(&tx, &item.object_id)?.source);
        }
    }
    let next_cursor = if more {
        items
            .last()
            .map(|item| serde_json::to_string(&(&item.object_id, &item.object_kind)))
            .transpose()
            .map_err(|_| invalid("无法生成搜索游标"))?
    } else {
        None
    };
    tx.commit()?;
    Ok(NoteSearchPage {
        items,
        next_cursor,
        index_state: "ready".to_owned(),
    })
}

#[cfg(test)]
#[path = "legacy_search_tests.rs"]
mod tests;
