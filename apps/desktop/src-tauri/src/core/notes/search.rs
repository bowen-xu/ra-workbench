use std::sync::{Arc, Mutex};

use rusqlite::{Connection, InterruptHandle, params};

use super::{
    error::NotesError,
    model::{NoteSearchPage, NoteSearchQuery},
    reading_note_repository::reading_note_from,
};

#[derive(Clone, Default)]
pub(crate) struct NoteIndexManager {
    interrupt: Arc<Mutex<Option<InterruptHandle>>>,
}

impl NoteIndexManager {
    pub(crate) fn begin(&self, interrupt: InterruptHandle) -> Result<(), NotesError> {
        let mut active = self
            .interrupt
            .lock()
            .map_err(|_| NotesError::IndexFailed("任务状态不可用".to_owned()))?;
        if active.is_some() {
            return Err(NotesError::IndexBusy);
        }
        *active = Some(interrupt);
        Ok(())
    }

    pub(crate) fn finish(&self) {
        if let Ok(mut active) = self.interrupt.lock() {
            *active = None;
        }
    }

    pub(crate) fn cancel(&self) -> Result<bool, NotesError> {
        let active = self
            .interrupt
            .lock()
            .map_err(|_| NotesError::IndexFailed("任务状态不可用".to_owned()))?;
        if let Some(interrupt) = active.as_ref() {
            interrupt.interrupt();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub(crate) fn index_state_from(connection: &Connection) -> Result<String, NotesError> {
    Ok(connection.query_row(
        "select state from note_index_state where id = 1",
        [],
        |row| row.get(0),
    )?)
}

pub(crate) fn rebuild_note_index_from(connection: &mut Connection) -> Result<(), NotesError> {
    connection.execute(
        "update note_index_state set state = 'rebuilding', last_error_code = null where id = 1",
        [],
    )?;
    let result = (|| -> Result<(), NotesError> {
        let transaction = connection.transaction()?;
        transaction.execute("delete from note_search", [])?;
        {
            let mut statement = transaction.prepare(
                "select p.id, p.title, p.updated_at,
                        coalesce((select group_concat(b.plain_text, char(10)) from note_blocks b where b.page_id = p.id), ''),
                        coalesce((select group_concat(t.name, ' ') from note_page_tags x join tags t on t.id = x.tag_id where x.page_id = p.id), ''),
                        coalesce((select group_concat(r.title, ' ') from project_note_pages x join research_projects r on r.id = x.project_id where x.page_id = p.id), '')
                 from note_pages p where not exists(select 1 from reading_note_sources s where s.page_id=p.id)",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (id, title, updated_at, body, tags, projects) in rows {
                transaction.execute(
                    "insert into note_search
                     (object_id, object_kind, title, body, citation_text, paper_title, creators, tags, project_titles, source_state, updated_at)
                     values (?1, 'note_page', ?2, ?3, '', '', '', ?4, ?5, '', ?6)",
                    params![id, title, body, tags, projects, updated_at],
                )?;
            }
        }
        let note_ids = transaction
            .prepare("select block_id from reading_note_sources order by block_id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let notes = note_ids
            .iter()
            .map(|id| reading_note_from(&transaction, id))
            .collect::<Result<Vec<_>, _>>()?;
        for note in notes {
            let creators: String = transaction.query_row(
                "select coalesce(group_concat(case when c.literal_name <> '' then c.literal_name else trim(c.family_name || ' ' || c.given_name) end, ' '), '')
                 from item_creators x join creators c on c.id = x.creator_id where x.item_id = ?1",
                [&note.source.paper_id],
                |row| row.get(0),
            )?;
            let tags: String = transaction.query_row(
                "select coalesce(group_concat(t.name, ' '), '')
                 from reading_note_tags x join tags t on t.id = x.tag_id where x.block_id = ?1",
                [&note.id],
                |row| row.get(0),
            )?;
            let projects = if note.project_ids.is_empty() {
                String::new()
            } else {
                let placeholders = std::iter::repeat_n("?", note.project_ids.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "select coalesce(group_concat(title, ' '), '') from research_projects where id in ({placeholders})"
                );
                let mut statement = transaction.prepare(&sql)?;
                statement.query_row(rusqlite::params_from_iter(note.project_ids.iter()), |row| {
                    row.get(0)
                })?
            };
            transaction.execute(
                "insert into note_search
                 (object_id, object_kind, title, body, citation_text, paper_title, creators, tags, project_titles, source_state, updated_at)
                 values (?1, 'reading_note', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![note.id, note.source.paper_title, note.body, note.source.quote_exact,
                    note.source.paper_title, creators, tags, projects, note.source.state, note.updated_at],
            )?;
        }
        transaction.execute(
            "update note_index_state set state = 'ready', last_rebuilt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), last_error_code = null where id = 1",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    })();
    if let Err(error) = result {
        let is_cancelled = matches!(
            &error,
            NotesError::Database(rusqlite::Error::SqliteFailure(failure, _))
                if failure.code == rusqlite::ErrorCode::OperationInterrupted
        );
        let state = if is_cancelled { "dirty" } else { "failed" };
        let code = if is_cancelled {
            "cancelled"
        } else {
            "rebuild_failed"
        };
        let _ = connection.execute(
            "update note_index_state set state = ?1, last_error_code = ?2 where id = 1",
            params![state, code],
        );
        return if is_cancelled {
            Err(NotesError::IndexCancelled)
        } else {
            Err(error)
        };
    }
    Ok(())
}

pub(crate) fn search_notes_from(
    connection: &Connection,
    query: NoteSearchQuery,
) -> Result<NoteSearchPage, NotesError> {
    super::catalog::legacy_search::search_from(connection, query)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::storage::migrate;

    use super::*;

    fn query(status: &str, limit: usize, cursor: Option<String>) -> NoteSearchQuery {
        NoteSearchQuery {
            search: None,
            status: Some(status.to_owned()),
            object_kinds: Some(vec!["note_page".to_owned()]),
            project_id: None,
            tag_id: None,
            paper_id: None,
            note_type: None,
            source_state: None,
            is_favorite: None,
            limit: Some(limit),
            cursor,
        }
    }

    fn insert_page(connection: &Connection, id: &str, status: &str) {
        connection.execute(
            "insert into note_pages
             (id, title, position, status, is_favorite, revision, schema_version, created_at, updated_at)
             values (?1, ?1, 0, ?2, 0, 0, 1, 'now', 'now')",
            params![id, status],
        ).expect("page");
        connection.execute(
            "insert into note_search
             (object_id, object_kind, title, body, citation_text, paper_title, creators, tags, project_titles, source_state, updated_at)
             values (?1, 'note_page', ?1, '', '', '', '', '', '', '', 'now')",
            [id],
        ).expect("search row");
    }

    #[test]
    fn scans_past_filtered_chunks_and_uses_an_opaque_stable_cursor() {
        let mut connection = Connection::open_in_memory().expect("database");
        migrate(&mut connection).expect("migrate");
        for index in 0..105 {
            insert_page(&connection, &format!("archived-{index:03}"), "archived");
        }
        for id in ["z-active-1", "z-active-2", "z-active-3"] {
            insert_page(&connection, id, "active");
        }
        let first = search_notes_from(&connection, query("active", 2, None)).expect("first page");
        assert_eq!(
            first
                .items
                .iter()
                .map(|item| item.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["z-active-1", "z-active-2"]
        );
        let second = search_notes_from(&connection, query("active", 2, first.next_cursor))
            .expect("second page");
        assert_eq!(
            second
                .items
                .iter()
                .map(|item| item.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["z-active-3"]
        );
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn rejects_malformed_search_cursors() {
        let mut connection = Connection::open_in_memory().expect("database");
        migrate(&mut connection).expect("migrate");
        let error = match search_notes_from(
            &connection,
            query("active", 10, Some("not-json".to_owned())),
        ) {
            Ok(_) => panic!("malformed cursor should fail"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("游标"));
    }

    #[test]
    fn failed_rebuild_keeps_the_previous_index_and_reports_failure() {
        let mut connection = Connection::open_in_memory().expect("database");
        migrate(&mut connection).expect("migrate");
        insert_page(&connection, "preserved", "active");
        connection
            .execute("drop table note_page_tags", [])
            .expect("break derived dependency");
        assert!(rebuild_note_index_from(&mut connection).is_err());
        let count: i64 = connection
            .query_row(
                "select count(*) from note_search where object_id = 'preserved'",
                [],
                |row| row.get(0),
            )
            .expect("preserved index");
        assert_eq!(count, 1);
        assert_eq!(
            index_state_from(&connection).expect("index state"),
            "failed"
        );
    }

    #[test]
    fn index_manager_owns_one_cancellable_job() {
        let connection = Connection::open_in_memory().expect("database");
        let manager = NoteIndexManager::default();
        manager
            .begin(connection.get_interrupt_handle())
            .expect("first job");
        assert!(matches!(
            manager.begin(connection.get_interrupt_handle()),
            Err(NotesError::IndexBusy)
        ));
        assert!(manager.cancel().expect("cancel active"));
        manager.finish();
        assert!(!manager.cancel().expect("no active job"));
    }
}
