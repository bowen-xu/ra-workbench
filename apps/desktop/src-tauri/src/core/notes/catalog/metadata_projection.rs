//! Deferred, resumable metadata-only preparation for records predating 0203.
use super::NotesError;
use rusqlite::{Connection, Transaction};

pub(super) fn snapshot<'a>(
    connection: &'a Connection,
    scope: &str,
) -> Result<Transaction<'a>, NotesError> {
    snapshot_with_budget(connection, scope, None)
}

pub(super) fn snapshot_with_budget<'a>(
    connection: &'a Connection,
    scope: &str,
    batches: Option<usize>,
) -> Result<Transaction<'a>, NotesError> {
    snapshot_between(connection, scope, || Ok(()), batches)
}

#[cfg(test)]
fn snapshot_with<'a>(
    connection: &'a Connection,
    scope: &str,
    between: impl FnMut() -> rusqlite::Result<()>,
) -> Result<Transaction<'a>, NotesError> {
    snapshot_between(connection, scope, between, None)
}

fn snapshot_between<'a>(
    connection: &'a Connection,
    scope: &str,
    mut between: impl FnMut() -> rusqlite::Result<()>,
    mut batches: Option<usize>,
) -> Result<Transaction<'a>, NotesError> {
    for _ in 0..3 {
        prepare(connection, scope, &mut batches)?;
        between()?;
        let tx = connection.unchecked_transaction()?;
        let pending:bool=tx.query_row("select exists(select 1 from note_catalog_metadata_pending m join note_page_catalog c on c.id=m.page_id where case when ?1='live' then c.status<>'trashed' else c.status=?1 end)",[scope],|row|row.get(0))?;
        if !pending {
            let (errors,cause):(i64,Option<String>)=tx.query_row("select count(*),min(e.cause) from note_catalog_projection_errors e join note_page_catalog c on c.id=e.page_id where case when ?1='live' then c.status<>'trashed' else c.status=?1 end",[scope],|row|Ok((row.get(0)?,row.get(1)?)))?;
            if errors > 0 {
                return Err(NotesError::IndexFailed(format!(
                    "note_catalog_invalid_text：{errors} 个笔记块无法建立搜索投影，请检查原始文档后重试。原因：{}",
                    cause.unwrap_or_default()
                )));
            }
            return Ok(tx);
        }
        // A restore/reclassification after preparation can bring pending old
        // metadata into scope. The result and readiness must share a snapshot.
        tx.rollback()?;
    }
    Err(NotesError::CatalogChanged {
        current_revision: super::read_revision(connection)?,
    })
}

fn prepare(
    connection: &Connection,
    scope: &str,
    batches: &mut Option<usize>,
) -> Result<(), NotesError> {
    loop {
        let tx = connection.unchecked_transaction()?;
        let ids = {
            let mut statement = tx.prepare(
                "select m.page_id from note_catalog_metadata_pending m join note_page_catalog c on c.id=m.page_id where case when ?1='live' then c.status<>'trashed' else c.status=?1 end order by m.page_id limit 100",
            )?;
            statement
                .query_map([scope], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        if ids.is_empty() {
            tx.commit()?;
            return Ok(());
        }
        // A command owns at most one durable batch. Return control to its
        // caller before dispatching another, so a hidden query can stop here.
        if *batches == Some(0) {
            return Err(NotesError::CatalogPreparing);
        }
        for id in ids {
            tx.execute("insert into note_catalog_metadata_text(page_id,search_text,folded_text)
                select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id=?1
                on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text",[&id])?;
            tx.execute(
                "delete from note_catalog_metadata_pending where page_id=?1",
                [&id],
            )?;
        }
        // All rows in this batch observe current metadata in the same durable
        // transaction. Failure retains this batch for retry, not partial success.
        tx.commit()?;
        if let Some(remaining) = batches {
            *remaining -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn old_database() -> Connection {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 202);
        db.execute_batch("insert into note_pages(id,title,status,created_at,updated_at) values('candidate','note','archived','2026-10-24T00:00:00Z','2026-10-24T00:00:00Z');
          insert into tags(id,name) values('tag','expectedtoken');
          insert into note_page_tags(page_id,tag_id) values('candidate','tag');").unwrap();
        crate::storage::migrate(&mut db).unwrap();
        db
    }
    #[test]
    fn a_restore_between_preparation_and_snapshot_cannot_publish_partial_search() {
        let db = old_database();
        let mut attempts = 0;
        let tx = snapshot_with(&db, "active", || {
            attempts += 1;
            if attempts == 1 {
                db.execute(
                    "update note_pages set status='active' where id='candidate'",
                    [],
                )?;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(attempts, 2);
        let found:i64=tx.query_row("select count(*) from note_catalog_metadata_search where note_catalog_metadata_search match 'expectedtoken'",[],|row|row.get(0)).unwrap();
        assert_eq!(found, 1);
        tx.commit().unwrap();
    }
    #[test]
    fn repeated_invalidations_are_bounded_and_never_claim_ready() {
        let db = old_database();
        let mut attempts = 0;
        let result = snapshot_with(&db, "archived", || {
            attempts += 1;
            db.execute(
                "insert into note_catalog_metadata_pending(page_id) values('candidate')",
                [],
            )?;
            Ok(())
        });
        assert!(matches!(result, Err(NotesError::CatalogChanged { .. })));
        assert_eq!(attempts, 3);
    }
}
