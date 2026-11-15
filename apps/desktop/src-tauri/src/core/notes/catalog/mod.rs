//! Paged canonical page/reading queries. Browse reads only metadata; optional
//! search reads persistent derived indexes, never editable documents or HBS state.
#[cfg(test)]
#[path = "consistency_tests.rs"]
mod consistency_tests;
#[path = "legacy_search.rs"]
pub(super) mod legacy_search;
#[path = "metadata_projection.rs"]
mod metadata_projection;
#[cfg(test)]
#[path = "metadata_search_tests.rs"]
mod metadata_search_tests;
#[path = "model.rs"]
mod model;
#[path = "order_keys.rs"]
mod order_keys;
#[cfg(test)]
#[path = "order_upgrade_tests.rs"]
mod order_upgrade_tests;
#[cfg(test)]
#[path = "projection_boundary_tests.rs"]
mod projection_boundary_tests;
#[path = "search.rs"]
mod search;
#[cfg(test)]
#[path = "search_tests.rs"]
mod search_tests;
#[path = "sorted.rs"]
mod sorted;
#[cfg(test)]
#[path = "sorted_tests.rs"]
mod sorted_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[path = "text_projection.rs"]
mod text_projection;
#[cfg(test)]
#[path = "text_projection_tests.rs"]
mod text_projection_tests;

use rusqlite::Connection;
use tauri::State;

use super::error::{NotesCommandError, NotesError};
use crate::storage::AppStore;
use model::{PageCatalogCursor, PageCatalogItem, PageCatalogOwner, PageCatalogTarget};
pub use model::{PageCatalogPage, PageCatalogQuery};
pub(crate) use text_projection::{document_text, register_projection_functions};

fn invalid(message: &str) -> NotesError {
    NotesError::InvalidInput(message.to_owned())
}

/// One source generation across browse, sorting and search. Metadata-only
/// checkpoints must also observe text-only changes without reading any text.
fn read_revision(connection: &rusqlite::Connection) -> rusqlite::Result<i64> {
    connection.query_row(
        "select (select generation from note_page_catalog_state where id=1)+(select generation from note_catalog_search_state where id=1)",
        [],
        |row| row.get(0),
    )
}

pub(super) fn row_item(
    row: &rusqlite::Row<'_>,
    snippet: String,
) -> rusqlite::Result<PageCatalogItem> {
    let id: String = row.get(0)?;
    let category: String = row.get(2)?;
    let source: Option<String> = row.get(3)?;
    let label: String = row.get(4)?;
    let position: i64 = row.get(8)?;
    let owner = source
        .as_ref()
        .filter(|_| category == "dependent")
        .map(|source| PageCatalogOwner {
            kind: "highlight",
            id: source.clone(),
            label: label.clone(),
        });
    let target = source.map_or_else(
        || PageCatalogTarget::Page {
            page_id: id.clone(),
        },
        |note_id| PageCatalogTarget::Reading {
            page_id: id.clone(),
            note_id,
        },
    );
    Ok(PageCatalogItem {
        id: format!("page:{id}"),
        title: row.get(1)?,
        snippet,
        category,
        owner,
        source_label: (!label.is_empty()).then_some(label),
        updated_at: row.get(5)?,
        status: row.get(6)?,
        is_favorite: row.get::<_, i64>(7)? != 0,
        favorite_position: (position != i64::MAX).then_some(position),
        target,
    })
}

fn query_sql(favorites: bool, category: bool) -> String {
    let category = if category { " and category=?5" } else { "" };
    let select = "select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,catalog_updated_ms from note_page_catalog";
    if favorites {
        format!(
            "{select} where status=?1 and is_favorite=1 and (favorite_position,id)>(?2,?3){category} order by favorite_position,id limit ?4"
        )
    } else {
        // The timestamp is descending but the ID tie-breaker is ascending. Two
        // indexed ranges avoid rescanning every earlier ID on tied timestamps.
        format!(
            "{select} where status=?1 and catalog_updated_ms=?2 and id>?3{category} union all {select} where status=?1 and catalog_updated_ms<?2{category} order by catalog_updated_ms desc,id limit ?4"
        )
    }
}

#[cfg(test)]
pub(crate) fn query_from(
    connection: &Connection,
    query: PageCatalogQuery,
) -> Result<PageCatalogPage, NotesError> {
    query_with_preparation_budget(connection, query, None)
}

fn query_with_preparation_budget(
    connection: &Connection,
    query: PageCatalogQuery,
    preparation_batches: Option<usize>,
) -> Result<PageCatalogPage, NotesError> {
    if !matches!(query.category.as_str(), "all" | "independent" | "dependent")
        || !matches!(
            query.view.as_str(),
            "recent" | "favorites" | "archive" | "trash"
        )
        || query.limit.is_some_and(|limit| !(1..=200).contains(&limit))
        || query
            .search
            .as_ref()
            .is_some_and(|search| search.len() > 4096)
    {
        return Err(invalid(
            "note_catalog_query_invalid：笔记列表查询范围无效。",
        ));
    }
    if let Some(sort) = &query.sort {
        sorted::ordering(sort)?;
    }
    if let Some(search) = &query.search
        && !search::terms(search)?.is_empty()
    {
        return search::query_search(connection, query, preparation_batches);
    }
    if query.sort.is_some() {
        return sorted::query_from(connection, query);
    }
    let transaction = connection.unchecked_transaction()?;
    order_keys::check_ready(&transaction, &query, false)?;
    let revision = read_revision(&transaction)?;
    if query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > 8192)
    {
        return Err(invalid("note_catalog_cursor_invalid：笔记列表游标无效。"));
    }
    let cursor: Option<PageCatalogCursor> = query
        .cursor
        .as_deref()
        .map(|raw| {
            serde_json::from_str(raw)
                .map_err(|_| invalid("note_catalog_cursor_invalid：笔记列表游标无效。"))
        })
        .transpose()?;
    if let Some(cursor) = &cursor {
        if cursor.schema != 1 || cursor.category != query.category || cursor.view != query.view {
            return Err(invalid(
                "note_catalog_cursor_scope：笔记列表游标不属于当前查询。",
            ));
        }
        if cursor.revision != revision {
            return Err(NotesError::CatalogChanged {
                current_revision: revision,
            });
        }
    }
    let status = match query.view.as_str() {
        "archive" => "archived",
        "trash" => "trashed",
        _ => "active",
    };
    let favorites = query.view == "favorites";
    let sql = query_sql(favorites, query.category != "all");
    let mut statement = transaction.prepare(&sql)?;
    let limit = query.limit.unwrap_or(20);
    let after = if favorites {
        rusqlite::types::Value::Integer(cursor.as_ref().map_or(-1, |value| value.favorite_position))
    } else {
        rusqlite::types::Value::Integer(match &cursor {
            Some(cursor) => order_keys::utc_millis(&cursor.updated_at).ok_or_else(|| {
                invalid("note_catalog_cursor_time_invalid：笔记列表游标时间无效。")
            })?,
            None => i64::MAX,
        })
    };
    statement.raw_bind_parameter(1, status)?;
    statement.raw_bind_parameter(2, after)?;
    statement.raw_bind_parameter(3, cursor.as_ref().map_or("", |value| value.id.as_str()))?;
    statement.raw_bind_parameter(4, (limit + 1) as i64)?;
    if query.category != "all" {
        statement.raw_bind_parameter(5, &query.category)?;
    }
    let mut rows = statement.raw_query();
    let mut items = Vec::new();
    let mut positions = Vec::new();
    while let Some(row) = rows.next()? {
        positions.push((row.get::<_, String>(0)?, row.get::<_, i64>(8)?));
        items.push(row_item(row, String::new())?);
    }
    let more = items.len() > limit;
    items.truncate(limit);
    positions.truncate(limit);
    let next_cursor = if more {
        let last = items
            .last()
            .ok_or_else(|| invalid("note_catalog_empty_page"))?;
        let (id, favorite_position) = positions
            .last()
            .ok_or_else(|| invalid("note_catalog_empty_page"))?;
        Some(
            serde_json::to_string(&PageCatalogCursor {
                schema: 1,
                revision,
                category: query.category,
                view: query.view,
                updated_at: last.updated_at.clone(),
                id: id.clone(),
                favorite_position: *favorite_position,
            })
            .map_err(|_| invalid("note_catalog_cursor_encode_failed"))?,
        )
    } else {
        None
    };
    drop(rows);
    drop(statement);
    transaction.commit()?;
    Ok(PageCatalogPage {
        items,
        revision,
        next_cursor,
    })
}

#[tauri::command]
/// Query the committed catalog off the main thread. Returned identities and
/// catalog generations are read projections, never authorization or edit versions.
pub async fn query_note_page_catalog(
    store: State<'_, AppStore>,
    query: PageCatalogQuery,
) -> Result<PageCatalogPage, NotesCommandError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = store.connection().map_err(NotesError::from)?;
        query_with_preparation_budget(&connection, query, Some(1)).map_err(NotesCommandError::from)
    })
    .await
    .map_err(|error| NotesError::IndexFailed(format!("note_catalog_query_task_failed：{error}")))?
}
