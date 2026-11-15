//! Substring search over persisted text, never canonical bodies or editor state.
use super::{NotesError, invalid, model::*};
use rusqlite::{Connection, types::Value};
use serde::{Deserialize, Serialize};
#[cfg(test)]
#[path = "search_plan_tests.rs"]
mod tests;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchCursor {
    schema: u8,
    revision: i64,
    terms: Vec<String>,
    view: String,
    category: String,
    id: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<PageCatalogSort>,
}

pub(super) fn terms(text: &str) -> Result<Vec<String>, NotesError> {
    research_note_query::note_query_terms(text).map_err(|cause| invalid(&cause))
}

fn scan_term(term: &str) -> bool {
    term.chars().count() < 3 || term.contains('\0')
}

/// Shared persisted-text matches. Parameters 1..=4 belong to the caller;
/// folded terms follow their FTS equivalents, starting at parameter 5.
pub(super) fn matching_pages_sql(terms: &[String]) -> String {
    let mut hits = Vec::new();
    for (index, term) in terms.iter().enumerate() {
        let parameter = index + 5;
        let raw_parameter = parameter + terms.len();
        // FTS5 cannot match trigrams for one/two Unicode scalar values. Those
        // terms scan the persisted normalized projection, never note JSON.
        if scan_term(term) {
            hits.push(format!("select page_id,{index} as term,block_id from note_catalog_blocks where instr(folded_text,?{raw_parameter})>0"));
            hits.push(format!("select page_id,{index} as term,null as block_id from note_catalog_metadata_text where instr(folded_text,?{raw_parameter})>0"));
        } else {
            hits.push(format!("select b.page_id,{index} as term,b.block_id from note_catalog_block_search f join note_catalog_blocks b on b.rowid=f.rowid where note_catalog_block_search match ?{parameter} and instr(b.folded_text,?{raw_parameter})>0"));
            hits.push(format!("select m.page_id,{index} as term,null as block_id from note_catalog_metadata_search f join note_catalog_metadata_text m on m.rowid=f.rowid where note_catalog_metadata_search match ?{parameter} and instr(m.folded_text,?{raw_parameter})>0"));
        }
    }
    format!(
        "with hits as ({}), matches as (select page_id,min(block_id) as preview_block from hits group by page_id having count(distinct term)={})",
        hits.join(" union all "),
        terms.len()
    )
}

fn search_sql(terms: &[String], order: Option<super::sorted::Ordering>) -> String {
    let order = order.unwrap_or(super::sorted::Ordering {
        column: "catalog_updated_ms",
        index: "time_down",
        descending_time: true,
    });
    format!("{}
        select c.id,c.title,c.category,c.source_id,c.source_label,c.updated_at,c.status,c.is_favorite,c.favorite_position,coalesce(substr(b.search_text,1,240),'') as snippet
        from matches m join note_page_catalog c on c.id=m.page_id left join note_catalog_blocks b on b.block_id=m.preview_block
        where (case when ?1='trash' then c.status='trashed' else c.status<>'trashed' end)
        and (c.{column}{comparison}?2 or (c.{column}=?2 and c.id>?3)) order by c.{column} {direction},c.id limit ?4",matching_pages_sql(terms),column=order.column,comparison=if order.descending_time {"<"}else{">"},direction=if order.descending_time {"desc"}else{"asc"})
}

fn parameters(
    query: &PageCatalogQuery,
    terms: &[String],
    cursor: Option<&SearchCursor>,
    limit: usize,
) -> Vec<Value> {
    let mut parameters = vec![
        Value::Text(query.view.clone()),
        Value::Integer(
            cursor
                .and_then(|cursor| super::order_keys::utc_millis(&cursor.updated_at))
                .unwrap_or(i64::MAX),
        ),
        Value::Text(cursor.map_or_else(String::new, |cursor| cursor.id.clone())),
        Value::Integer((limit + 1) as i64),
    ];
    parameters.extend(terms.iter().map(|term| {
        Value::Text(if scan_term(term) {
            term.clone()
        } else {
            format!("\"{}\"", term.replace('"', "\"\""))
        })
    }));
    parameters.extend(terms.iter().cloned().map(Value::Text));
    parameters
}

pub(super) fn query_search(
    connection: &Connection,
    query: PageCatalogQuery,
    preparation_batches: Option<usize>,
) -> Result<PageCatalogPage, NotesError> {
    let terms = terms(query.search.as_deref().unwrap_or_default())?;
    if terms.is_empty() {
        return Err(invalid("note_catalog_search_invalid：搜索词不能为空。"));
    }
    if query
        .cursor
        .as_ref()
        .is_some_and(|value| value.len() > 8192)
    {
        return Err(invalid("note_catalog_cursor_invalid：笔记搜索游标无效。"));
    }
    let cursor: Option<SearchCursor> = query
        .cursor
        .as_deref()
        .map(|raw| {
            serde_json::from_str(raw)
                .map_err(|_| invalid("note_catalog_cursor_invalid：笔记搜索游标无效。"))
        })
        .transpose()?;
    if let Some(cursor) = &cursor
        && (cursor.schema != if query.sort.is_some() { 3 } else { 2 }
            || cursor.terms != terms
            || cursor.view != query.view
            || cursor.category != query.category
            || cursor.sort != query.sort)
    {
        return Err(invalid(
            "note_catalog_cursor_scope：笔记搜索游标不属于当前查询。",
        ));
    }
    let tx = super::metadata_projection::snapshot_with_budget(
        connection,
        if query.view == "trash" {
            "trashed"
        } else {
            "live"
        },
        preparation_batches,
    )?;
    super::order_keys::check_ready(&tx, &query, true)?;
    let revision = super::read_revision(&tx)?;
    if let Some(cursor) = &cursor
        && cursor.revision != revision
    {
        return Err(NotesError::CatalogChanged {
            current_revision: revision,
        });
    }
    let limit = query.limit.unwrap_or(20);
    let order = super::sorted::ordering(&query.sort.clone().unwrap_or(PageCatalogSort {
        field: "updatedAt".to_owned(),
        direction: "descending".to_owned(),
    }))?;
    let after: Value = if let Some(cursor) = &cursor {
        tx.query_row(
            &format!("select {} from note_page_catalog where id=?1", order.column),
            [&cursor.id],
            |row| row.get(0),
        )?
    } else if order.column == "catalog_updated_ms" {
        Value::Integer(if order.descending_time {
            i64::MAX
        } else {
            i64::MIN
        })
    } else {
        Value::Blob(Vec::new())
    };
    let mut bindings = parameters(&query, &terms, cursor.as_ref(), limit);
    bindings[1] = after;
    let mut statement = tx.prepare(&search_sql(&terms, Some(order)))?;
    let mut rows = statement.query(rusqlite::params_from_iter(bindings))?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        items.push(super::row_item(row, row.get(9)?)?);
    }
    let more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = if more {
        let item = items
            .last()
            .ok_or_else(|| invalid("note_catalog_empty_page"))?;
        Some(
            serde_json::to_string(&SearchCursor {
                schema: if query.sort.is_some() { 3 } else { 2 },
                sort: query.sort,
                revision,
                terms,
                view: query.view,
                category: query.category,
                id: item.id[5..].to_owned(),
                updated_at: item.updated_at.clone(),
            })
            .map_err(|_| invalid("note_catalog_cursor_encode_failed"))?,
        )
    } else {
        None
    };
    drop(rows);
    drop(statement);
    tx.commit()?;
    Ok(PageCatalogPage {
        items,
        revision,
        next_cursor,
    })
}
