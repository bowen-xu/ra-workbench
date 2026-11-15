use super::*;
use crate::agent::repository::create_conversation;
use research_agent_core::conversation::ConversationId;

pub(super) fn setup(db: &mut Connection) -> Uuid {
    crate::storage_migration_tests::migrate_through(db, 94);
    let id = Uuid::new_v4();
    create_conversation(db, ConversationId::from_uuid(id), "queue fixture").unwrap();
    id
}
fn input() -> QueuedInput {
    QueuedInput {
        user_input: vec![],
        origin: None,
        id: Uuid::new_v4(),
        text: "synthetic input".into(),
    }
}

#[test]
fn deleted_input_identity_cannot_be_reused_and_conversations_are_isolated() {
    let mut db = Connection::open_in_memory().unwrap();
    let scope = setup(&mut db);
    let other = Uuid::new_v4();
    create_conversation(&mut db, ConversationId::from_uuid(other), "other").unwrap();
    let item = input();
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        0,
        QueueEdit::Add(item.clone()),
        |_| Ok(()),
    )
    .unwrap();
    assert!(
        read_checked(&mut db, other, |_| Ok(()))
            .unwrap()
            .inputs()
            .is_empty()
    );
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        1,
        QueueEdit::Remove(item.id),
        |_| Ok(()),
    )
    .unwrap();
    for (conversation, revision) in [(scope, 2), (other, 0)] {
        assert!(matches!(
            apply_checked(
                &mut db,
                conversation,
                Uuid::new_v4(),
                revision,
                QueueEdit::Add(item.clone()),
                |_| Ok(())
            ),
            Err(Error::Queue(QueueError::Conflict))
        ));
    }
}

#[test]
fn reopens_preparing_queue_and_retries_original_receipt_without_reapplying() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    let scope = setup(&mut db);
    let request = Uuid::new_v4();
    let item = input();
    let added = apply_checked(
        &mut db,
        scope,
        request,
        0,
        QueueEdit::Add(item.clone()),
        |_| Ok(()),
    )
    .unwrap();
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        1,
        QueueEdit::Prepare(item.id),
        |_| Ok(()),
    )
    .unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
    assert_eq!(queue.revision(), 2);
    assert_eq!(queue.preparing(), Some(item.id));
    assert_eq!(
        apply_checked(&mut db, scope, request, 0, QueueEdit::Add(item), |_| Ok(())).unwrap(),
        added
    );
    assert_eq!(read_checked(&mut db, scope, |_| Ok(())).unwrap(), queue);
}

#[test]
fn failed_receipt_write_rolls_back_queue_and_same_request_can_retry() {
    let mut db = Connection::open_in_memory().unwrap();
    let scope = setup(&mut db);
    db.execute_batch("create trigger fail_receipt before insert on agent_input_queue_receipts begin select raise(abort,'fixture'); end;").unwrap();
    let request = Uuid::new_v4();
    let item = input();
    assert!(
        apply_checked(
            &mut db,
            scope,
            request,
            0,
            QueueEdit::Add(item.clone()),
            |_| Ok(())
        )
        .is_err()
    );
    assert!(
        read_checked(&mut db, scope, |_| Ok(()))
            .unwrap()
            .inputs()
            .is_empty()
    );
    db.execute_batch("drop trigger fail_receipt").unwrap();
    assert_eq!(
        apply_checked(&mut db, scope, request, 0, QueueEdit::Add(item), |_| Ok(()))
            .unwrap()
            .inputs()
            .len(),
        1
    );
}

#[test]
fn authorization_precedes_replay_and_conflicting_retries_or_revisions_fail() {
    let mut db = Connection::open_in_memory().unwrap();
    let scope = setup(&mut db);
    let request = Uuid::new_v4();
    let item = input();
    apply_checked(
        &mut db,
        scope,
        request,
        0,
        QueueEdit::Add(item.clone()),
        |_| Ok(()),
    )
    .unwrap();
    assert!(matches!(
        apply_checked(
            &mut db,
            scope,
            request,
            0,
            QueueEdit::Add(item.clone()),
            |_| Err(Error::Closed)
        ),
        Err(Error::Closed)
    ));
    assert!(matches!(
        apply_checked(&mut db, scope, request, 0, QueueEdit::Pause, |_| Ok(())),
        Err(Error::Queue(QueueError::Conflict))
    ));
    assert!(matches!(
        apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            0,
            QueueEdit::Remove(item.id),
            |_| Ok(())
        ),
        Err(Error::Queue(QueueError::Conflict))
    ));
    db.execute(
        "update agent_conversations set archived=1 where id=?1",
        [scope.to_string()],
    )
    .unwrap();
    assert!(matches!(
        apply_checked(&mut db, scope, request, 0, QueueEdit::Add(item), |_| Ok(())),
        Err(Error::Closed)
    ));
}

#[test]
fn corrupt_snapshots_and_uncoordinated_admission_fail_closed() {
    let mut db = Connection::open_in_memory().unwrap();
    let scope = setup(&mut db);
    let item = input();
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        0,
        QueueEdit::Add(item.clone()),
        |_| Ok(()),
    )
    .unwrap();
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        1,
        QueueEdit::Prepare(item.id),
        |_| Ok(()),
    )
    .unwrap();
    assert!(matches!(
        apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            2,
            QueueEdit::Admitted(item.id),
            |_| Ok(())
        ),
        Err(Error::Queue(QueueError::Invalid))
    ));
    db.execute("update agent_input_queues set snapshot_json=json_set(snapshot_json,'$.version',99) where conversation_id=?1", [scope.to_string()]).unwrap();
    assert!(matches!(
        read_checked(&mut db, scope, |_| Ok(())),
        Err(Error::Corrupt)
    ));
}

#[test]
fn individual_pause_survives_reopen_and_exact_receipt_retry_with_a_nonfirst_claim() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    let scope = setup(&mut db);
    let first = input();
    let second = input();
    for (revision, item) in [first.clone(), second.clone()].into_iter().enumerate() {
        apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            revision as u64,
            QueueEdit::Add(item),
            |_| Ok(()),
        )
        .unwrap();
    }
    let request = Uuid::new_v4();
    let paused = apply_checked(
        &mut db,
        scope,
        request,
        2,
        QueueEdit::PauseInput(first.id),
        |_| Ok(()),
    )
    .unwrap();
    let preparing = apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        3,
        QueueEdit::Prepare(second.id),
        |_| Ok(()),
    )
    .unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    assert_eq!(read_checked(&mut db, scope, |_| Ok(())).unwrap(), preparing);
    assert_eq!(
        apply_checked(
            &mut db,
            scope,
            request,
            2,
            QueueEdit::PauseInput(first.id),
            |_| Ok(())
        )
        .unwrap(),
        paused
    );
    assert!(matches!(
        apply_checked(
            &mut db,
            scope,
            request,
            2,
            QueueEdit::ResumeInput(first.id),
            |_| Ok(())
        ),
        Err(Error::Queue(QueueError::Conflict))
    ));
    assert_eq!(read_checked(&mut db, scope, |_| Ok(())).unwrap(), preparing);
}
