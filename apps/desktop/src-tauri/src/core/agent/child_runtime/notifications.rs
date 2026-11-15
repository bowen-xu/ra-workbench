//! Child scheduling invalidation follows committed SQL changes. The callback
//! runs after releasing both the SQLite mutex and the data admission fence.

use crate::{data_interface::SharedDatabase, storage::AppStore};
use rusqlite::Connection;
use uuid::Uuid;

use super::super::repository::subtasks;

pub(super) fn parent(db: &Connection, child: Uuid) -> Result<Option<Uuid>, subtasks::Error> {
    Ok(subtasks::find_by_child(db, child)?.map(|record| record.parent_conversation_id))
}

/// Return a parent only when the operation actually changed scheduling state.
/// Transactions must commit before returning; failures and no-ops emit nothing.
pub(super) fn write<T, E>(
    store: &AppStore,
    unavailable: E,
    operation: impl FnOnce(&mut Connection) -> Result<(T, Option<Uuid>), E>,
) -> Result<T, E> {
    write_and_notify(
        &store.shared_agent_connection(),
        unavailable,
        operation,
        |parent| store.notify_agent_subtasks_changed(Some(parent)),
    )
}

fn write_and_notify<T, E>(
    database: &SharedDatabase,
    unavailable: E,
    operation: impl FnOnce(&mut Connection) -> Result<(T, Option<Uuid>), E>,
    notify: impl FnOnce(Uuid),
) -> Result<T, E> {
    let (result, parent) = {
        let mut db = database.lock().map_err(|_| unavailable)?;
        operation(&mut db)?
    };
    if let Some(parent) = parent {
        notify(parent);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_interface::IntoDatabase;
    use research_agent_core::subtasks::SubtaskSpec;
    use std::sync::{Arc, Mutex};

    fn fixture() -> (Arc<Mutex<Connection>>, SharedDatabase, SubtaskSpec) {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table agent_subtasks (child_conversation_id text primary key,parent_conversation_id text not null,parent_context_snapshot_id text not null,child_context_snapshot_id text not null,depth integer not null,model_calls integer not null,tool_calls integer not null,approved_servers_json text not null default '[]',status text not null,created_at text not null default 'now',finished_at text);").unwrap();
        let db = Arc::new(Mutex::new(db));
        let database = db.clone().into_database();
        let spec = SubtaskSpec {
            parent_conversation_id: Uuid::new_v4(),
            child_conversation_id: Uuid::new_v4(),
            parent_context_snapshot_id: Uuid::new_v4(),
            child_context_snapshot_id: Uuid::new_v4(),
            depth: 1,
            model_calls: 1,
            tool_calls: 1,
            approved_servers: Default::default(),
        };
        (db, database, spec)
    }

    #[test]
    fn committed_transitions_notify_after_unlock_and_noops_do_not() {
        let (raw, database, spec) = fixture();
        let child = spec.child_conversation_id;
        let parent = spec.parent_conversation_id;
        let notices = Mutex::new(Vec::new());
        let notify = |id| {
            let db = raw
                .try_lock()
                .expect("notification must run after SQL unlock");
            let record = subtasks::find_by_child(&db, child).unwrap().unwrap();
            notices.lock().unwrap().push((id, record.status));
        };
        write_and_notify(
            &database,
            subtasks::Error::Storage,
            |db| {
                subtasks::insert(db, &spec)?;
                Ok(((), Some(parent)))
            },
            notify,
        )
        .unwrap();
        for status in [
            subtasks::Status::RecoveryRequired,
            subtasks::Status::Completed,
        ] {
            write_and_notify(
                &database,
                subtasks::Error::Storage,
                |db| {
                    subtasks::finish(db, child, status)?;
                    Ok(((), Some(parent)))
                },
                notify,
            )
            .unwrap();
            if status == subtasks::Status::RecoveryRequired {
                for release in [false, false, true, true, false] {
                    write_and_notify(
                        &database,
                        subtasks::Error::Storage,
                        |db| {
                            let changed = if release {
                                subtasks::release_resume_claim(db, child)?
                            } else {
                                subtasks::claim_for_resume(db, child)?
                            };
                            Ok(((), changed.then_some(parent)))
                        },
                        notify,
                    )
                    .unwrap();
                }
                let retry = Uuid::new_v4();
                for previous in [
                    spec.child_context_snapshot_id,
                    spec.child_context_snapshot_id,
                ] {
                    write_and_notify(
                        &database,
                        subtasks::Error::Storage,
                        |db| {
                            let changed = subtasks::bind_retry_context(db, child, previous, retry)?;
                            Ok(((), changed.then_some(parent)))
                        },
                        notify,
                    )
                    .unwrap();
                }
            }
        }
        let result = write_and_notify(
            &database,
            subtasks::Error::Storage,
            |db| {
                subtasks::finish(db, child, subtasks::Status::Failed)?;
                Ok(((), Some(parent)))
            },
            notify,
        );
        assert_eq!(result, Err(subtasks::Error::Invalid));
        let notices = notices.into_inner().unwrap();
        assert_eq!(notices.len(), 7);
        assert!(notices.iter().all(|(id, _)| *id == parent));
        assert_eq!(notices.last().unwrap().1, subtasks::Status::Completed);
    }

    #[test]
    fn failed_transaction_rolls_back_without_notification() {
        let (raw, database, spec) = fixture();
        let result = write_and_notify(
            &database,
            (),
            |db| {
                let tx = db.transaction().map_err(|_| ())?;
                subtasks::insert(&tx, &spec).map_err(|_| ())?;
                Err::<((), Option<Uuid>), _>(())
            },
            |_| panic!("failed transaction cannot notify"),
        );
        assert_eq!(result, Err(()));
        assert!(
            subtasks::find_by_child(&raw.lock().unwrap(), spec.child_conversation_id)
                .unwrap()
                .is_none()
        );
    }
}
