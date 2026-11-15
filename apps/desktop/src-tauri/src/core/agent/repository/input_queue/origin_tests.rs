use super::*;
use research_agent_core::conversation::BindingKind;
use serde_json::json;

fn origin() -> ConversationBinding {
    ConversationBinding {
        kind: BindingKind::ReadingNote,
        resource_id: Uuid::new_v4().to_string(),
    }
}

#[test]
fn migration_preserves_legacy_rows_and_terminal_pause_keeps_new_origins() {
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, TurnOutcome},
    };
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 93);
    let conversation = Uuid::new_v4();
    let id = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation),
        "migration",
    )
    .unwrap();
    let legacy=json!({"version":1,"conversation":conversation,"revision":1,"paused":false,"inputs":[[id,"legacy"]],"preparing":null}).to_string();
    db.execute(
        "insert into agent_input_queues(conversation_id,revision,snapshot_json) values(?1,1,?2)",
        params![conversation.to_string(), legacy],
    )
    .unwrap();
    db.execute_batch(include_str!(
        "../../../../../migrations/0094_agent_queue_origin_snapshot.sql"
    ))
    .unwrap();
    let raw: String = db
        .query_row(
            "select snapshot_json from agent_input_queues where conversation_id=?1",
            [conversation.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, legacy);
    let event = AgentEvent::new(
        ConversationId::from_uuid(conversation),
        1,
        0,
        Uuid::new_v4(),
        EventPayload::TurnFinished {
            turn_id: Uuid::new_v4(),
            outcome: TurnOutcome::Interrupted,
        },
    )
    .unwrap();
    crate::agent::repository::append_batch(
        &mut db,
        ConversationId::from_uuid(conversation),
        Uuid::new_v4(),
        0,
        &[event],
    )
    .unwrap();
    let paused = read_checked(&mut db, conversation, |_| Ok(())).unwrap();
    assert!(paused.paused());
    assert_eq!(paused.inputs()[0].origin, None);
    let source = origin();
    let saved = apply_checked(
        &mut db,
        conversation,
        Uuid::new_v4(),
        paused.revision(),
        QueueEdit::Add(QueuedInput {
            user_input: vec![],
            id: Uuid::new_v4(),
            text: "new".into(),
            origin: Some(source.clone()),
        }),
        |_| Ok(()),
    )
    .unwrap();
    let resumed = apply_checked(
        &mut db,
        conversation,
        Uuid::new_v4(),
        saved.revision(),
        QueueEdit::Resume,
        |_| Ok(()),
    )
    .unwrap();
    let event = AgentEvent::new(
        ConversationId::from_uuid(conversation),
        2,
        0,
        Uuid::new_v4(),
        EventPayload::TurnFinished {
            turn_id: Uuid::new_v4(),
            outcome: TurnOutcome::Failed,
        },
    )
    .unwrap();
    crate::agent::repository::append_batch(
        &mut db,
        ConversationId::from_uuid(conversation),
        Uuid::new_v4(),
        1,
        &[event],
    )
    .unwrap();
    let paused = read_checked(&mut db, conversation, |_| Ok(())).unwrap();
    assert!(paused.paused());
    assert_eq!(paused.revision(), resumed.revision() + 1);
    assert_eq!(paused.inputs()[0].origin, None);
    assert_eq!(paused.inputs()[1].origin, Some(source));
}

#[test]
fn snapshots_preserve_origins_and_legacy_snapshots_never_infer_them() {
    let conversation = Uuid::new_v4();
    let id = Uuid::new_v4();
    let source = origin();
    let legacy = json!({"version":1,"conversation":conversation,"revision":3,"paused":true,"inputs":[[id,"legacy"]],"preparing":id});
    let restored = Snapshot::decode(&legacy.to_string(), conversation).unwrap();
    assert_eq!(restored.inputs()[0].origin, None);
    let saved = Snapshot::encode(&restored).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&saved).unwrap()["version"],
        2
    );
    assert_eq!(Snapshot::decode(&saved, conversation).unwrap(), restored);
    let queue = InputQueue::new(conversation)
        .unwrap()
        .apply(
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                id,
                text: "new".into(),
                origin: Some(source),
            }),
        )
        .unwrap();
    assert_eq!(
        Snapshot::decode(&Snapshot::encode(&queue).unwrap(), conversation).unwrap(),
        queue
    );
}

#[test]
fn corrupt_origins_cannot_silently_be_ignored_or_relabel_another_input() {
    let conversation = Uuid::new_v4();
    let id = Uuid::new_v4();
    let source = origin();
    let base = json!({"version":2,"conversation":conversation,"revision":1,"paused":false,"inputs":[[id,"text"]],"preparing":null,"origins":[[id,source]]});
    for origins in [
        json!([[Uuid::new_v4(), source]]),
        json!([[id, source], [id, source]]),
        json!([[id,{"kind":"note","resourceId":Uuid::nil()}]]),
        json!([[id,{"kind":"unknown","resourceId":Uuid::new_v4()}]]),
    ] {
        let mut value = base.clone();
        value["origins"] = origins;
        assert!(Snapshot::decode(&value.to_string(), conversation).is_err());
    }
    let mut value = base;
    value["version"] = json!(1);
    assert!(Snapshot::decode(&value.to_string(), conversation).is_err());
}

#[test]
fn origin_and_receipt_commit_atomically_and_survive_file_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("origin.sqlite");
    let mut db = Connection::open(&path).unwrap();
    let conversation = tests::setup(&mut db);
    let request = Uuid::new_v4();
    let input = QueuedInput {
        user_input: vec![],
        id: Uuid::new_v4(),
        text: "source text".into(),
        origin: Some(origin()),
    };
    db.execute_batch("create trigger reject_receipt before insert on agent_input_queue_receipts begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        apply_checked(
            &mut db,
            conversation,
            request,
            0,
            QueueEdit::Add(input.clone()),
            |_| Ok(())
        )
        .is_err()
    );
    assert!(
        read_checked(&mut db, conversation, |_| Ok(()))
            .unwrap()
            .inputs()
            .is_empty()
    );
    db.execute_batch("drop trigger reject_receipt").unwrap();
    let saved = apply_checked(
        &mut db,
        conversation,
        request,
        0,
        QueueEdit::Add(input.clone()),
        |_| Ok(()),
    )
    .unwrap();
    drop(db);
    let mut db = Connection::open(path).unwrap();
    assert_eq!(
        read_checked(&mut db, conversation, |_| Ok(())).unwrap(),
        saved
    );
    assert_eq!(
        apply_checked(
            &mut db,
            conversation,
            request,
            0,
            QueueEdit::Add(input.clone()),
            |_| Ok(())
        )
        .unwrap(),
        saved
    );
    let mut relabeled = input;
    relabeled.origin = Some(origin());
    assert!(matches!(
        apply_checked(
            &mut db,
            conversation,
            request,
            0,
            QueueEdit::Add(relabeled),
            |_| Ok(())
        ),
        Err(Error::Queue(QueueError::Conflict))
    ));
}

#[test]
fn legacy_receipt_retry_preserves_unknown_origin_and_does_not_rewrite_the_queue() {
    let mut db = Connection::open_in_memory().unwrap();
    let conversation = tests::setup(&mut db);
    let request = Uuid::new_v4();
    let mut input = QueuedInput {
        user_input: vec![],
        id: Uuid::new_v4(),
        text: "legacy text".into(),
        origin: None,
    };
    let saved = apply_checked(
        &mut db,
        conversation,
        request,
        0,
        QueueEdit::Add(input.clone()),
        |_| Ok(()),
    )
    .unwrap();
    // An actual v1 result from a pre-upgrade release, not a new origin-less result.
    db.execute("update agent_input_queue_receipts set result_json=json_set(result_json,'$.version',1) where request_id=?1",[request.to_string()]).unwrap();
    input.origin = Some(origin());
    assert_eq!(
        apply_checked(
            &mut db,
            conversation,
            request,
            0,
            QueueEdit::Add(input),
            |_| Ok(())
        )
        .unwrap(),
        saved
    );
    assert_eq!(
        read_checked(&mut db, conversation, |_| Ok(()))
            .unwrap()
            .inputs()[0]
            .origin,
        None
    );
}
