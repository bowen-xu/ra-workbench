//! A persistent steering handoff owns its queue input across windows.
use super::*;
use crate::agent::repository::SqlRepositoryError;

pub(super) fn require_unclaimed(
    db: &Connection,
    conversation: Uuid,
    input: Uuid,
) -> Result<(), QueueSqlError> {
    let exists: bool = db.query_row("select exists(select 1 from sqlite_master where type='table' and name='agent_steering_handoffs')", [], |row| row.get(0))?;
    if !exists {
        // Historical upgrade fixtures and pre-220 databases cannot contain a
        // handoff. A missing table after migration is a damaged schema instead.
        let migrated: bool = db.query_row(
            "select exists(select 1 from schema_migrations where version=220)",
            [],
            |row| row.get(0),
        )?;
        return if migrated {
            Err(SqlRepositoryError::Domain(Error::Corrupt))
        } else {
            Ok(())
        };
    }
    let claimed: bool = db.query_row("select exists(select 1 from agent_steering_handoffs where conversation_id=?1 and queue_input_id=?2)", params![conversation.to_string(), input.to_string()], |row| row.get(0))?;
    if claimed {
        Err(SqlRepositoryError::Domain(QueueError::Busy.into()))
    } else {
        Ok(())
    }
}

pub(super) fn require_edit_unclaimed(
    db: &Connection,
    conversation: Uuid,
    edit: &QueueEdit,
) -> Result<(), QueueSqlError> {
    let input = match edit {
        QueueEdit::Update { id, .. }
        | QueueEdit::Remove(id)
        | QueueEdit::Prepare(id)
        | QueueEdit::Admitted(id) => Some(*id),
        QueueEdit::Restore { source_id, .. } => Some(*source_id),
        _ => None,
    };
    if let Some(input) = input {
        require_unclaimed(db, conversation, input)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_schema_is_compatible_but_missing_migrated_table_fails_closed() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table schema_migrations(version integer primary key)")
            .unwrap();
        let conversation = Uuid::new_v4();
        let input = Uuid::new_v4();
        assert!(require_unclaimed(&db, conversation, input).is_ok());
        db.execute("insert into schema_migrations values(220)", [])
            .unwrap();
        assert!(matches!(
            require_unclaimed(&db, conversation, input),
            Err(SqlRepositoryError::Domain(Error::Corrupt))
        ));
        db.execute_batch(
            "create table agent_steering_handoffs(conversation_id text,queue_input_id text)",
        )
        .unwrap();
        db.execute(
            "insert into agent_steering_handoffs values(?1,?2)",
            params![conversation.to_string(), input.to_string()],
        )
        .unwrap();
        assert!(matches!(
            require_unclaimed(&db, conversation, input),
            Err(SqlRepositoryError::Domain(Error::Queue(QueueError::Busy)))
        ));
        assert!(require_unclaimed(&db, Uuid::new_v4(), input).is_ok());
        assert!(require_unclaimed(&db, conversation, Uuid::new_v4()).is_ok());
    }
}
