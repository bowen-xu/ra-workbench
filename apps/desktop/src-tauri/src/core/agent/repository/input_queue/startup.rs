//! Before admitting work in a new application process, persisted queues pause.
use super::*;

/// Run once at native service construction, never on UI mount or conversation open.
/// Preserves prepared heads and receipts for explicit recovery. All updates commit
/// together; a corrupt queue aborts startup recovery rather than silently dropping it.
pub fn pause_on_startup(db: &mut Connection) -> Result<usize, Error> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    let mut statement = tx.prepare("select conversation_id,revision,snapshot_json from agent_input_queues order by conversation_id")
        .map_err(|_| Error::Storage)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| Error::Storage)?;
    let mut changed = 0;
    for row in rows {
        let (id, revision, json) = row.map_err(|_| Error::Storage)?;
        let conversation = Uuid::parse_str(&id).map_err(|_| Error::Corrupt)?;
        let queue = Snapshot::decode(&json, conversation)?;
        if queue.revision() != revision {
            return Err(Error::Corrupt);
        }
        if queue.paused() {
            continue;
        }
        let next = queue.apply(revision, QueueEdit::Pause)?;
        tx.execute(
            "update agent_input_queues set revision=?2,snapshot_json=?3 where conversation_id=?1",
            params![id, next.revision(), Snapshot::encode(&next)?],
        )
        .map_err(|_| Error::Storage)?;
        changed += 1;
    }
    drop(statement);
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(changed)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::repository::create_conversation;
    use research_agent_core::conversation::ConversationId;
    #[test]
    fn restart_pauses_all_queues_without_releasing_claims_or_dropping_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup.sqlite");
        let mut db = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 94);
        let mut identities = Vec::new();
        for _ in 0..3 {
            let scope = Uuid::new_v4();
            let input = Uuid::new_v4();
            create_conversation(&mut db, ConversationId::from_uuid(scope), "startup").unwrap();
            apply_checked(
                &mut db,
                scope,
                Uuid::new_v4(),
                0,
                QueueEdit::Add(QueuedInput {
                    user_input: vec![],
                    origin: None,
                    id: input,
                    text: "retained".into(),
                }),
                |_| Ok(()),
            )
            .unwrap();
            apply_checked(
                &mut db,
                scope,
                Uuid::new_v4(),
                1,
                QueueEdit::Prepare(input),
                |_| Ok(()),
            )
            .unwrap();
            identities.push((scope, input));
        }
        drop(db);
        let mut db = Connection::open(path).unwrap();
        assert_eq!(pause_on_startup(&mut db).unwrap(), 3);
        assert_eq!(pause_on_startup(&mut db).unwrap(), 0);
        for (scope, input) in identities {
            let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
            assert!(queue.paused());
            assert_eq!(queue.revision(), 3);
            assert_eq!(queue.preparing(), Some(input));
            assert_eq!(queue.inputs()[0].text, "retained");
        }
    }
    #[test]
    fn failed_startup_update_rolls_back_every_queue() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 94);
        for index in 1..=2 {
            let scope = Uuid::from_u128(index);
            create_conversation(&mut db, ConversationId::from_uuid(scope), "startup").unwrap();
            apply_checked(
                &mut db,
                scope,
                Uuid::new_v4(),
                0,
                QueueEdit::Add(QueuedInput {
                    user_input: vec![],
                    origin: None,
                    id: Uuid::new_v4(),
                    text: "retained".into(),
                }),
                |_| Ok(()),
            )
            .unwrap();
        }
        db.execute_batch("create trigger reject_second before update on agent_input_queues when new.conversation_id='00000000-0000-0000-0000-000000000002' begin select raise(abort,'fixture'); end;").unwrap();
        assert!(pause_on_startup(&mut db).is_err());
        for index in 1..=2 {
            let queue = read_checked(&mut db, Uuid::from_u128(index), |_| Ok(())).unwrap();
            assert!(!queue.paused());
            assert_eq!(queue.revision(), 1);
        }
    }
}
