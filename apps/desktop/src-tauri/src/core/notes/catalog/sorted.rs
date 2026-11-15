use super::{NotesError, invalid, model::*, order_keys, row_item};
use rusqlite::{Connection, OptionalExtension, types::Value};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
pub(super) struct Ordering {
    pub column: &'static str,
    pub index: &'static str,
    pub descending_time: bool,
}

pub(super) fn ordering(sort: &PageCatalogSort) -> Result<Ordering, NotesError> {
    let down = match sort.direction.as_str() {
        "ascending" => false,
        "descending" => true,
        _ => return Err(invalid("note_catalog_sort_invalid：笔记排序方向无效。")),
    };
    let (column, index) = match (sort.field.as_str(), down) {
        ("title", false) => ("catalog_title_key", "title_up"),
        ("title", true) => ("catalog_title_down", "title_down"),
        ("category", false) => ("catalog_category_key", "category_up"),
        ("category", true) => ("catalog_category_down", "category_down"),
        ("source", false) => ("catalog_source_key", "source_up"),
        ("source", true) => ("catalog_source_down", "source_down"),
        ("updatedAt", false) => ("catalog_updated_ms", "time_up"),
        ("updatedAt", true) => ("catalog_updated_ms", "time_down"),
        _ => return Err(invalid("note_catalog_sort_invalid：笔记排序字段无效。")),
    };
    Ok(Ordering {
        column,
        index,
        descending_time: sort.field == "updatedAt" && down,
    })
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cursor {
    schema: u8,
    revision: i64,
    category: String,
    view: String,
    sort: PageCatalogSort,
    id: String,
}

pub(super) fn query_sql(order: Ordering, query: &PageCatalogQuery, after: bool) -> String {
    let favorite = query.view == "favorites";
    let suffix = if favorite {
        "favorite"
    } else if query.category == "all" {
        "all"
    } else {
        "category"
    };
    let index = if order.index == "time_down" && !favorite {
        if suffix == "all" {
            "note_catalog_recent_utc".to_owned()
        } else {
            "note_catalog_category_recent_utc".to_owned()
        }
    } else {
        format!("note_catalog_{}_{}", order.index, suffix)
    };
    let scope = format!(
        "status=?1{}{}",
        if query.category == "all" {
            ""
        } else {
            " and category=?2"
        },
        if favorite { " and is_favorite=1" } else { "" }
    );
    let select = format!(
        "select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,{} from note_page_catalog indexed by {index}",
        order.column
    );
    if order.descending_time && after {
        format!(
            "{select} where {scope} and {}=?4 and id>?5 union all {select} where {scope} and {}<?4 order by {} desc,id limit ?6",
            order.column, order.column, order.column
        )
    } else {
        let seek = if after {
            format!(" and ({},id)>(?4,?5)", order.column)
        } else {
            String::new()
        };
        format!(
            "{select} where {scope}{seek} order by {} {},id limit ?6",
            order.column,
            if order.descending_time { "desc" } else { "asc" }
        )
    }
}

pub(super) fn query_from(
    connection: &Connection,
    query: PageCatalogQuery,
) -> Result<PageCatalogPage, NotesError> {
    let sort = query
        .sort
        .clone()
        .ok_or_else(|| invalid("note_catalog_sort_required"))?;
    let order = ordering(&sort)?;
    let tx = connection.unchecked_transaction()?;
    order_keys::check_ready(&tx, &query, false)?;
    let revision = super::read_revision(&tx)?;
    if query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > 8192)
    {
        return Err(invalid("note_catalog_cursor_invalid：笔记列表游标无效。"));
    }
    let cursor: Option<Cursor> = query
        .cursor
        .as_deref()
        .map(|value| {
            serde_json::from_str(value)
                .map_err(|_| invalid("note_catalog_cursor_invalid：笔记列表游标无效。"))
        })
        .transpose()?;
    if let Some(cursor) = &cursor {
        if cursor.schema != 3
            || cursor.category != query.category
            || cursor.view != query.view
            || cursor.sort != sort
        {
            return Err(invalid(
                "note_catalog_cursor_scope：笔记列表游标不属于当前排序。",
            ));
        }
        if cursor.revision != revision {
            return Err(NotesError::CatalogChanged {
                current_revision: revision,
            });
        }
    }
    let after: Value = if let Some(cursor) = &cursor {
        tx.query_row(
            &format!("select {} from note_page_catalog where id=?1", order.column),
            [&cursor.id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| invalid("note_catalog_cursor_missing：排序游标对象不存在。"))?
    } else {
        Value::Null
    };
    let status = if query.view == "archive" {
        "archived"
    } else if query.view == "trash" {
        "trashed"
    } else {
        "active"
    };
    let limit = query.limit.unwrap_or(20);
    let sql = query_sql(order, &query, cursor.is_some());
    let mut statement = tx.prepare(&sql)?;
    statement.raw_bind_parameter(1, status)?;
    statement.raw_bind_parameter(2, &query.category)?;
    statement.raw_bind_parameter(4, after)?;
    statement.raw_bind_parameter(5, cursor.as_ref().map_or("", |cursor| cursor.id.as_str()))?;
    statement.raw_bind_parameter(6, (limit + 1) as i64)?;
    let mut rows = statement.raw_query();
    let mut items = Vec::new();
    let mut ids = Vec::new();
    while let Some(row) = rows.next()? {
        ids.push(row.get::<_, String>(0)?);
        items.push(row_item(row, String::new())?);
    }
    let more = items.len() > limit;
    items.truncate(limit);
    ids.truncate(limit);
    let next_cursor = if more {
        Some(
            serde_json::to_string(&Cursor {
                schema: 3,
                revision,
                category: query.category,
                view: query.view,
                sort,
                id: ids
                    .pop()
                    .ok_or_else(|| invalid("note_catalog_empty_page"))?,
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
