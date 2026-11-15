use super::tests::setup;
use super::*;
use research_agent_core::conversation::BindingKind;

#[test]
fn restores_full_resources_position_origin_and_pause_once_without_reusing_identity() {
    let mut db = Connection::open_in_memory().unwrap();
    let conversation = setup(&mut db);
    let original = QueuedInput {id: Uuid::new_v4(), text: "@review".into(),
        user_input: serde_json::from_value(serde_json::json!([
            {"type":"text","text":"@review","textElements":[{"startUtf16":0,"endUtf16":7,"itemIndex":1}]},
            {"type":"skill","name":"review","identity":{"environment_id":"app","source_id":"review/SKILL.md","revision":"v1"}}
        ])).unwrap(), origin: Some(ConversationBinding {kind: BindingKind::Note, resource_id: Uuid::new_v4().to_string()})};
    let apply = |db: &mut Connection, revision, edit| {
        apply_checked(db, conversation, Uuid::new_v4(), revision, edit, |_| Ok(())).unwrap()
    };
    apply(&mut db, 0, QueueEdit::Add(original.clone()));
    apply(&mut db, 1, QueueEdit::PauseInput(original.id));
    apply(&mut db, 2, QueueEdit::Remove(original.id));
    let tail = QueuedInput {
        id: Uuid::new_v4(),
        text: "tail".into(),
        user_input: vec![],
        origin: None,
    };
    apply(&mut db, 3, QueueEdit::Add(tail.clone()));
    let mut edited = original.clone();
    edited.id = Uuid::new_v4();
    edited.origin = None;
    let edit = QueueEdit::Restore {
        input: edited.clone(),
        source_id: original.id,
        index: 0,
        paused: false,
    };
    let request = Uuid::new_v4();
    let saved = apply_checked(&mut db, conversation, request, 4, edit.clone(), |_| Ok(())).unwrap();
    assert_eq!(saved.inputs()[0].origin, original.origin);
    assert_eq!(saved.inputs()[0].user_input, original.user_input);
    assert_eq!(saved.inputs()[1], tail);
    assert!(saved.input_paused(edited.id));
    assert_eq!(
        apply_checked(&mut db, conversation, request, 4, edit, |_| Ok(())).unwrap(),
        saved
    );
    let invalid = QueueEdit::Restore {
        input: QueuedInput {
            id: original.id,
            ..edited
        },
        source_id: original.id,
        index: 0,
        paused: false,
    };
    assert!(matches!(
        apply_checked(
            &mut db,
            conversation,
            Uuid::new_v4(),
            5,
            invalid,
            |_| Ok(())
        ),
        Err(Error::Queue(QueueError::Conflict))
    ));
}

#[test]
fn rejects_unknown_cross_conversation_or_still_queued_sources_without_mutation() {
    let mut db = Connection::open_in_memory().unwrap();
    let conversation = setup(&mut db);
    let source = QueuedInput {
        id: Uuid::new_v4(),
        text: "source".into(),
        user_input: vec![],
        origin: None,
    };
    apply_checked(
        &mut db,
        conversation,
        Uuid::new_v4(),
        0,
        QueueEdit::Add(source.clone()),
        |_| Ok(()),
    )
    .unwrap();
    let replacement = QueuedInput {
        id: Uuid::new_v4(),
        ..source.clone()
    };
    let restore = QueueEdit::Restore {
        input: replacement,
        source_id: source.id,
        index: 0,
        paused: false,
    };
    assert!(
        apply_checked(
            &mut db,
            conversation,
            Uuid::new_v4(),
            1,
            restore.clone(),
            |_| Ok(())
        )
        .is_err()
    );
    apply_checked(
        &mut db,
        conversation,
        Uuid::new_v4(),
        1,
        QueueEdit::Remove(source.id),
        |_| Ok(()),
    )
    .unwrap();
    let other = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut db,
        research_agent_core::conversation::ConversationId::from_uuid(other),
        "other",
    )
    .unwrap();
    assert!(
        apply_checked(&mut db, other, Uuid::new_v4(), 0, restore.clone(), |_| Ok(
            ()
        ))
        .is_err()
    );
    let unknown = QueueEdit::Restore {
        input: QueuedInput {
            id: Uuid::new_v4(),
            ..source
        },
        source_id: Uuid::new_v4(),
        index: 0,
        paused: false,
    };
    assert!(
        apply_checked(
            &mut db,
            conversation,
            Uuid::new_v4(),
            2,
            unknown,
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
}
