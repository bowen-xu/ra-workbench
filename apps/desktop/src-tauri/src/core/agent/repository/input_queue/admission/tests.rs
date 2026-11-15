use super::*;
use crate::agent::repository::create_conversation;
use research_agent_core::{conversation::ConversationId, protocol::ModelIdentity};
use sha2::Digest;

fn setup(db: &mut Connection) -> (Uuid, Uuid, SubmissionInput) {
    crate::storage_migration_tests::migrate_through(db, 95);
    let scope = Uuid::new_v4();
    let id = Uuid::new_v4();
    create_conversation(db, ConversationId::from_uuid(scope), "admission fixture").unwrap();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        text: "queued source request".into(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "test".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
    };
    apply_checked(
        db,
        scope,
        Uuid::new_v4(),
        0,
        QueueEdit::Add(QueuedInput {
            user_input: vec![],
            origin: None,
            id,
            text: input.text.clone(),
        }),
        |_| Ok(()),
    )
    .unwrap();
    apply_checked(db, scope, Uuid::new_v4(), 1, QueueEdit::Prepare(id), |_| {
        Ok(())
    })
    .unwrap();
    (scope, id, input)
}

#[test]
fn caller_owned_admission_rolls_back_queue_and_submission_together() {
    let mut db = Connection::open_in_memory().unwrap();
    let (scope, id, input) = setup(&mut db);
    let request = Uuid::new_v4();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let (_, next) = admit_in_tx(
        &tx,
        scope,
        request,
        2,
        id,
        input.clone(),
        &[],
        false,
        |_| Ok(()),
    )
    .unwrap();
    assert!(next.inputs().is_empty());
    tx.rollback().unwrap();
    let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
    assert_eq!(queue.preparing(), Some(id));
    assert_eq!(queue.inputs()[0].id, id);
    assert!(submissions::read(&db, scope, request).unwrap().is_none());
    let (_, committed) = admit_checked(&mut db, scope, request, 2, id, input, |_| Ok(())).unwrap();
    assert!(committed.inputs().is_empty());
}

#[test]
fn durable_retry_returns_same_turn_and_cannot_change_model_or_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("admission.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    let (scope, id, input) = setup(&mut db);
    let request = Uuid::new_v4();
    let (record, queue) =
        admit_checked(&mut db, scope, request, 2, id, input.clone(), |_| Ok(())).unwrap();
    assert!(queue.inputs().is_empty());
    drop(db);
    let mut db = Connection::open(path).unwrap();
    let (retry, snapshot) =
        admit_checked(&mut db, scope, request, 2, id, input.clone(), |_| Ok(())).unwrap();
    assert_eq!(retry, record);
    assert_eq!(snapshot, queue);
    let mut changed = input.clone();
    changed.model.model_id = "other".into();
    assert!(matches!(
        admit_checked(&mut db, scope, request, 2, id, changed, |_| Ok(())),
        Err(Error::Queue(QueueError::Conflict))
    ));
    assert!(matches!(
        admit_checked(&mut db, scope, request, 2, id, input, |_| Err(
            Error::Closed
        )),
        Err(Error::Closed)
    ));
    let count: i64 = db
        .query_row("select count(*) from agent_submissions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn later_pause_fences_replayed_admission_until_explicit_resume() {
    use research_agent_core::protocol::{AgentEvent, EventPayload, TaskKind};

    let mut db = Connection::open_in_memory().unwrap();
    let (scope, id, input) = setup(&mut db);
    let request = Uuid::new_v4();
    let (record, admitted) =
        admit_checked(&mut db, scope, request, 2, id, input.clone(), |_| Ok(())).unwrap();
    let paused = apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        admitted.revision(),
        QueueEdit::Pause,
        |_| Ok(()),
    )
    .unwrap();
    let (_, replayed) =
        admit_and_resume_checked(&mut db, scope, request, 2, id, input, |_| Ok(())).unwrap();
    assert_eq!(replayed, admitted);
    assert_eq!(read_checked(&mut db, scope, |_| Ok(())).unwrap(), paused);
    let conversation = ConversationId::from_uuid(scope);
    let event = AgentEvent::new(
        conversation,
        1,
        0,
        request,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
        },
    )
    .unwrap();
    let batch = Uuid::new_v4();
    assert!(
        crate::agent::repository::append_batch(
            &mut db,
            conversation,
            batch,
            0,
            std::slice::from_ref(&event)
        )
        .is_err()
    );
    let count: u64 = db
        .query_row(
            "select durable_count from agent_conversations where id=?1",
            [scope.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        paused.revision(),
        QueueEdit::Resume,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        crate::agent::repository::append_batch(&mut db, conversation, batch, 0, &[event]).unwrap(),
        1
    );
}

#[test]
fn failed_admission_rolls_back_submission_and_preserves_head_for_retry() {
    let mut db = Connection::open_in_memory().unwrap();
    let (scope, id, input) = setup(&mut db);
    let request = Uuid::new_v4();
    db.execute_batch("create trigger fail_admission before insert on agent_input_queue_admissions begin select raise(abort,'fixture'); end;").unwrap();
    assert!(admit_checked(&mut db, scope, request, 2, id, input.clone(), |_| Ok(())).is_err());
    assert!(submissions::read(&db, scope, request).unwrap().is_none());
    let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
    assert_eq!(queue.preparing(), Some(id));
    assert_eq!(queue.revision(), 2);
    db.execute_batch("drop trigger fail_admission").unwrap();
    assert!(admit_checked(&mut db, scope, request, 2, id, input, |_| Ok(())).is_ok());
}

#[test]
fn wrong_head_stale_revision_and_preexisting_submission_cannot_consume_input() {
    let mut db = Connection::open_in_memory().unwrap();
    let (scope, id, input) = setup(&mut db);
    for (revision, target, text) in [
        (1, id, input.text.clone()),
        (2, Uuid::new_v4(), input.text.clone()),
        (2, id, "changed".into()),
    ] {
        let mut candidate = input.clone();
        candidate.text = text;
        assert!(
            admit_checked(
                &mut db,
                scope,
                Uuid::new_v4(),
                revision,
                target,
                candidate,
                |_| Ok(())
            )
            .is_err()
        );
    }
    let request = Uuid::new_v4();
    let tx = db.transaction().unwrap();
    submissions::insert_in(&tx, scope, request, input.clone()).unwrap();
    tx.commit().unwrap();
    assert!(matches!(
        admit_checked(&mut db, scope, request, 2, id, input, |_| Ok(())),
        Err(Error::Queue(QueueError::Conflict))
    ));
    assert_eq!(
        read_checked(&mut db, scope, |_| Ok(()))
            .unwrap()
            .preparing(),
        Some(id)
    );
}

#[test]
fn image_manifest_is_committed_with_admission() {
    let mut db = Connection::open_in_memory().unwrap();
    let (scope, id, input) = setup(&mut db);
    let image = research_agent_core::submission::SubmissionImage {
        source_id: "paper-source:fixture".into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", sha2::Sha256::digest(b"\x89PNG\r\n\x1a\nfixture")),
        bytes: b"\x89PNG\r\n\x1a\nfixture".to_vec(),
    };
    let request = Uuid::new_v4();
    admit_checked_with_images(&mut db, scope, request, 2, id, input, &[image], |_| Ok(())).unwrap();
    let manifest = submissions::read_image_manifest(&db, scope, request).unwrap();
    assert_eq!(manifest.len(), 1);
    assert_eq!(manifest[0].source_id, "paper-source:fixture");
}

#[test]
fn admits_nonfirst_eligible_input_without_removing_the_paused_entry() {
    let mut db = Connection::open_in_memory().unwrap();
    let (scope, first, source) = setup(&mut db);
    let second = Uuid::new_v4();
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        2,
        QueueEdit::Release(first),
        |_| Ok(()),
    )
    .unwrap();
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        3,
        QueueEdit::Add(QueuedInput {
            id: second,
            text: source.text.clone(),
            user_input: vec![],
            origin: None,
        }),
        |_| Ok(()),
    )
    .unwrap();
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        4,
        QueueEdit::ResumeInput(second),
        |_| Ok(()),
    )
    .unwrap();
    apply_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        5,
        QueueEdit::Prepare(second),
        |_| Ok(()),
    )
    .unwrap();
    let (_, result) = admit_checked(
        &mut db,
        scope,
        Uuid::new_v4(),
        6,
        second,
        source,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(result.inputs().len(), 1);
    assert_eq!(result.inputs()[0].id, first);
    assert!(result.input_paused(first));
    assert!(!result.paused());
    assert!(result.next_input().is_none());
}
