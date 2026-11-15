use rusqlite::{Transaction, params};

use super::error::NotesError;

pub(super) fn sibling_ids(
    transaction: &Transaction<'_>,
    parent: Option<&str>,
    excluded: &str,
) -> Result<Vec<String>, NotesError> {
    let mut statement = transaction.prepare(
        "select id from note_pages
         where (parent_page_id = ?1 or (parent_page_id is null and ?1 is null)) and id <> ?2
         order by position, id",
    )?;
    Ok(statement
        .query_map(params![parent, excluded], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(super) fn apply_sibling_order(
    transaction: &Transaction<'_>,
    parent: Option<&str>,
    ids: &[String],
    moving_id: &str,
) -> Result<(), NotesError> {
    for (position, id) in ids.iter().enumerate() {
        transaction.execute(
            "update note_pages set
               parent_page_id = ?2,
               position = ?3,
               revision = revision + case when parent_page_id is not ?2 or position <> ?3 or id = ?4 then 1 else 0 end,
               updated_at = case when parent_page_id is not ?2 or position <> ?3 or id = ?4
                 then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else updated_at end
             where id = ?1",
            params![id, parent, position as i64, moving_id],
        )?;
    }
    Ok(())
}
