use super::error::{NotesCommandError, NotesError};
use crate::storage::AppStore;
use rusqlite::{Connection, params};
use serde::Deserialize;
use tauri::State;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReorderFavorites {
    pub page_ids: Vec<String>,
    pub expected_page_ids: Vec<String>,
}
pub(crate) fn list_from(connection: &Connection) -> Result<Vec<String>, NotesError> {
    let mut statement = connection.prepare("select f.page_id from note_favorite_order f join note_pages p on p.id=f.page_id
        where p.is_favorite=1 and not exists(select 1 from reading_note_sources s where s.page_id=p.id
          and (s.note_type='highlight' or (s.note_type='annotation' and trim(s.quote_exact)<>'')))
        order by f.position, f.page_id")?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?)
}
pub(crate) fn reorder_from(
    connection: &mut Connection,
    request: ReorderFavorites,
) -> Result<(), NotesError> {
    let transaction = connection.transaction()?;
    let current = list_from(&transaction)?;
    let mut before = current.clone();
    before.sort();
    let mut after = request.page_ids.clone();
    after.sort();
    if current != request.expected_page_ids || before != after {
        return Err(NotesError::InvalidInput(
            "收藏列表已变化，请刷新后重新排序。".to_owned(),
        ));
    }
    for (position, id) in request.page_ids.iter().enumerate() {
        transaction.execute(
            "update note_favorite_order set position=?2 where page_id=?1",
            params![id, position as i64],
        )?;
    }
    transaction.commit()?;
    Ok(())
}
#[tauri::command]
pub fn list_note_favorites(store: State<'_, AppStore>) -> Result<Vec<String>, NotesCommandError> {
    let connection = store.connection().map_err(NotesError::from)?;
    list_from(&connection).map_err(Into::into)
}
#[tauri::command]
pub fn reorder_note_favorites(
    store: State<'_, AppStore>,
    request: ReorderFavorites,
) -> Result<(), NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    reorder_from(&mut connection, request).map_err(Into::into)
}
