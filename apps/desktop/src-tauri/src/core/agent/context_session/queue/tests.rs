use super::*;
use research_agent_core::{
    conversation::ConversationId,
    input_queue::{QueueEdit, QueuedInput},
    protocol::ModelIdentity,
    submission::SubmissionInput,
};

fn fixture() -> (Connection, Uuid, Uuid, SubmissionInput) {
    fixture_with(Connection::open_in_memory().unwrap())
}

fn fixture_with(mut db: Connection) -> (Connection, Uuid, Uuid, SubmissionInput) {
    crate::storage::migrate(&mut db).unwrap();
    let id = Uuid::new_v4();
    crate::agent::repository::create_conversation(&mut db, ConversationId::from_uuid(id), "Queue")
        .unwrap();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        text: "queued".into(),
        user_input: vec![],
        skill_instructions: vec![],
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "test".into(),
            revision: None,
        },
        reasoning_effort: None,
        service_tier: None,
        context: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
    };
    let input_id = Uuid::new_v4();
    input_queue::apply_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        0,
        QueueEdit::Add(QueuedInput {
            id: input_id,
            text: input.text.clone(),
            user_input: vec![],
            origin: None,
        }),
        |db| authorize(db, id),
    )
    .unwrap();
    (db, id, input_id, input)
}

#[test]
fn managed_admission_commit_failure_blocks_owner_and_reopen_retains_original_head() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("queue.sqlite");
    let (mut sql, id, input_id, input) = fixture_with(Connection::open(&path).unwrap());
    let request = Uuid::new_v4();
    let (record, queue) =
        input_queue::prepare_preview_checked(&mut sql, id, request, 1, input_id, input, |db| {
            authorize(db, id)
        })
        .unwrap();
    sql.execute_batch("pragma foreign_keys=on; create table fixture_parent(id integer primary key); create table fixture_child(id integer references fixture_parent(id) deferrable initially deferred); create trigger fail_queue_commit after insert on agent_input_queue_admissions begin insert into fixture_child values(1); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    {
        let mut guard = db.lock().unwrap();
        assert!(matches!(
            admit_managed(
                &mut guard,
                &record,
                Some(Confirmation {
                    input_id,
                    expected_revision: queue.revision()
                }),
                false
            ),
            Err(Error::QueueAdmissionUnknown {
                phase: "commit",
                safe_cause: "sql_commit_outcome_unknown",
                sqlite_extended_code: Some(_),
                ..
            })
        ));
    }
    assert!(
        db.lock().is_err(),
        "unknown commit must fence the original owner"
    );
    drop(db);
    let sql = Connection::open(&path).unwrap();
    let reopened = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let mut guard = reopened.lock().unwrap();
    let proof_count: u64 = guard.query_row(
        "select count(*) from data_sql_operation_receipts where kind='agent.context.queue.admission'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        proof_count, 0,
        "failed admission cannot leave a proof behind"
    );
    let held = input_queue::read_checked(&mut guard, id, |db| authorize(db, id)).unwrap();
    assert_eq!(held.preparing(), Some(input_id));
    assert_eq!(held.inputs()[0].id, input_id);
    assert_eq!(held.revision(), queue.revision());
}

#[test]
fn managed_admission_known_conflict_keeps_owner_but_failed_rollback_fences_it() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let root = tempfile::tempdir().unwrap();
    let (mut sql, id, input_id, input) =
        fixture_with(Connection::open(root.path().join("queue.sqlite")).unwrap());
    let (record, queue) = input_queue::prepare_preview_checked(
        &mut sql,
        id,
        Uuid::new_v4(),
        1,
        input_id,
        input,
        |db| authorize(db, id),
    )
    .unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    {
        let mut guard = db.lock().unwrap();
        let stale = Confirmation {
            input_id,
            expected_revision: queue.revision() + 1,
        };
        assert!(matches!(
            admit_managed(&mut guard, &record, Some(stale), false),
            Err(Error::ConfirmationMismatch)
        ));
    }
    {
        let mut guard = db.lock().unwrap();
        guard.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
        let stale = Confirmation {
            input_id,
            expected_revision: queue.revision() + 1,
        };
        assert!(matches!(
            admit_managed(&mut guard, &record, Some(stale), false),
            Err(Error::QueueAdmissionUnknown {
                phase: "rollback",
                safe_cause: "sql_rollback_failed",
                ..
            })
        ));
    }
    assert!(
        db.lock().is_err(),
        "failed rollback must fence the original owner"
    );
}

#[test]
fn managed_admission_proves_resume_policy_and_replays_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("queue.sqlite");
    let (mut sql, id, input_id, input) = fixture_with(Connection::open(&path).unwrap());
    let request_id = Uuid::new_v4();
    let (record, queue) =
        input_queue::prepare_preview_checked(&mut sql, id, request_id, 1, input_id, input, |db| {
            authorize(db, id)
        })
        .unwrap();
    let confirmation = Confirmation {
        input_id,
        expected_revision: queue.revision(),
    };
    let status_request = crate::agent::context_session::start::Request {
        conversation_id: id,
        request_id,
        context_snapshot_id: record.context_snapshot_id,
        queue: Some(confirmation),
        resume_queue: false,
    };
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    {
        let mut guard = db.lock().unwrap();
        admit_managed(&mut guard, &record, Some(confirmation), false).unwrap();
        assert!(matches!(
            admit_managed(&mut guard, &record, Some(confirmation), true),
            Err(Error::ConfirmationMismatch)
        ));
    }
    assert!(matches!(
        db.inspect_context_start(&status_request),
        crate::data_interface::ContextStartInspection::Admitted(_)
    ));
    let mut changed = status_request.clone();
    changed.resume_queue = true;
    assert!(matches!(
        db.inspect_context_start(&changed),
        crate::data_interface::ContextStartInspection::Unconfirmed(_)
    ));
    drop(db);
    let db = crate::data_interface::Database::open(
        Connection::open(&path).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let mut guard = db.lock().unwrap();
    admit_managed(&mut guard, &record, Some(confirmation), false).unwrap();
    assert!(matches!(
        admit_managed(&mut guard, &record, Some(confirmation), true),
        Err(Error::ConfirmationMismatch)
    ));
    let state = input_queue::read_checked(&mut guard, id, |db| authorize(db, id)).unwrap();
    assert!(state.inputs().is_empty());
    assert_eq!(state.revision(), queue.revision() + 1);
}

#[test]
fn managed_admission_does_not_invent_proof_for_legacy_admission() {
    let root = tempfile::tempdir().unwrap();
    let (mut sql, id, input_id, input) =
        fixture_with(Connection::open(root.path().join("queue.sqlite")).unwrap());
    let request_id = Uuid::new_v4();
    let (record, queue) =
        input_queue::prepare_preview_checked(&mut sql, id, request_id, 1, input_id, input, |db| {
            authorize(db, id)
        })
        .unwrap();
    let confirmation = Confirmation {
        input_id,
        expected_revision: queue.revision(),
    };
    admit(&mut sql, &record, Some(confirmation), false).unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let request = crate::agent::context_session::start::Request {
        conversation_id: id,
        request_id,
        context_snapshot_id: record.context_snapshot_id,
        queue: Some(confirmation),
        resume_queue: false,
    };
    assert!(matches!(
        db.inspect_context_start(&request),
        crate::data_interface::ContextStartInspection::Unconfirmed(_)
    ));
    let mut guard = db.lock().unwrap();
    assert!(matches!(
        admit_managed(&mut guard, &record, Some(confirmation), false),
        Err(Error::Unavailable)
    ));
}

#[test]
fn managed_admission_receipt_failure_rolls_back_queue_removal() {
    let root = tempfile::tempdir().unwrap();
    let (mut sql, id, input_id, input) =
        fixture_with(Connection::open(root.path().join("queue.sqlite")).unwrap());
    let (record, queue) = input_queue::prepare_preview_checked(
        &mut sql,
        id,
        Uuid::new_v4(),
        1,
        input_id,
        input,
        |db| authorize(db, id),
    )
    .unwrap();
    sql.execute_batch("create trigger fail_context_queue_proof before insert on data_sql_operation_receipts when new.kind='agent.context.queue.admission' begin select raise(abort,'fixture'); end;").unwrap();
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let mut guard = db.lock().unwrap();
    assert!(matches!(
        admit_managed(
            &mut guard,
            &record,
            Some(Confirmation {
                input_id,
                expected_revision: queue.revision(),
            }),
            false
        ),
        Err(Error::Unavailable)
    ));
    let state = input_queue::read_checked(&mut guard, id, |db| authorize(db, id)).unwrap();
    assert_eq!(state.preparing(), Some(input_id));
    assert_eq!(state.revision(), queue.revision());
    let admissions: u64 = guard
        .query_row(
            "select count(*) from agent_input_queue_admissions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(admissions, 0);
}

#[test]
fn managed_resume_is_atomic_and_exact_retry_does_not_resume_later_pause() {
    let root = tempfile::tempdir().unwrap();
    let (mut sql, id, input_id, input) =
        fixture_with(Connection::open(root.path().join("queue.sqlite")).unwrap());
    let (record, queue) = input_queue::prepare_preview_checked(
        &mut sql,
        id,
        Uuid::new_v4(),
        1,
        input_id,
        input,
        |db| authorize(db, id),
    )
    .unwrap();
    let paused = input_queue::apply_checked(
        &mut sql,
        id,
        Uuid::new_v4(),
        queue.revision(),
        QueueEdit::Pause,
        |db| authorize(db, id),
    )
    .unwrap();
    let confirmation = Confirmation {
        input_id,
        expected_revision: paused.revision(),
    };
    let db = crate::data_interface::Database::open(sql, &root.path().join("log")).unwrap();
    let mut guard = db.lock().unwrap();
    admit_managed(&mut guard, &record, Some(confirmation), true).unwrap();
    let state = input_queue::read_checked(&mut guard, id, |db| authorize(db, id)).unwrap();
    assert!(!state.paused());
    let paused = input_queue::apply_checked(
        &mut guard,
        id,
        Uuid::new_v4(),
        state.revision(),
        QueueEdit::Pause,
        |db| authorize(db, id),
    )
    .unwrap();
    admit_managed(&mut guard, &record, Some(confirmation), true).unwrap();
    assert!(matches!(
        admit_managed(&mut guard, &record, Some(confirmation), false),
        Err(Error::ConfirmationMismatch)
    ));
    let state = input_queue::read_checked(&mut guard, id, |db| authorize(db, id)).unwrap();
    assert!(state.paused());
    assert_eq!(state.revision(), paused.revision());
}

#[test]
fn context_queue_edit_preserves_and_validates_expected_epoch() {
    let epoch = Uuid::new_v4();
    let mut value = serde_json::json!({"conversationId": Uuid::new_v4(), "requestId": Uuid::new_v4(),
        "expectedRevision": 0, "expectedEpoch": epoch, "edit": {"type": "resume"}});
    let request: EditRequest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(request.into_intent().unwrap().expected_epoch, Some(epoch));
    value["expectedEpoch"] = serde_json::json!(Uuid::nil());
    let request: EditRequest = serde_json::from_value(value).unwrap();
    assert!(matches!(request.into_intent(), Err(CommandError::Invalid)));
}

#[test]
fn independent_queue_freezes_consumes_and_replays_exactly_once() {
    let (mut db, id, input_id, input) = fixture();
    let request = Uuid::new_v4();
    let (record, queue) =
        input_queue::prepare_preview_checked(&mut db, id, request, 1, input_id, input, |db| {
            authorize(db, id)
        })
        .unwrap();
    let confirmation = Confirmation {
        input_id,
        expected_revision: queue.revision(),
    };
    assert!(matches!(
        admit(&mut db, &record, None, false),
        Err(Error::ConfirmationMismatch)
    ));
    let restored = restore_preview(&mut db, record.clone()).unwrap();
    assert_eq!(restored.queue.unwrap().input_id, input_id);
    admit(&mut db, &record, Some(confirmation), false).unwrap();
    admit(&mut db, &record, Some(confirmation), false).unwrap();
    let queue = input_queue::read_checked(&mut db, id, |db| authorize(db, id)).unwrap();
    assert!(queue.inputs().is_empty());
    assert_eq!(queue.revision(), 3);
    assert_eq!(
        restore_preview(&mut db, record)
            .unwrap()
            .queue
            .unwrap()
            .expected_revision,
        2
    );
}

#[test]
fn queue_cannot_bypass_archive_binding_or_another_conversation() {
    let (mut db, id, _, _) = fixture();
    assert!(input_queue::read_checked(&mut db, Uuid::new_v4(), |db| authorize(db, id)).is_err());
    db.execute(
        "update agent_conversations set archived=1 where id=?1",
        [id.to_string()],
    )
    .unwrap();
    assert!(input_queue::read_checked(&mut db, id, |db| authorize(db, id)).is_err());
    db.execute(
        "update agent_conversations set archived=0 where id=?1",
        [id.to_string()],
    )
    .unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'manuscript','other')",[id.to_string()]).unwrap();
    assert!(input_queue::read_checked(&mut db, id, |db| authorize(db, id)).is_err());
}

#[test]
fn queue_preparation_rejects_changed_input_and_admission_failure_retains_head() {
    let (mut db, id, input_id, mut input) = fixture();
    let request = Uuid::new_v4();
    input.text = "changed".into();
    assert!(
        input_queue::prepare_preview_checked(
            &mut db,
            id,
            request,
            1,
            input_id,
            input.clone(),
            |db| authorize(db, id)
        )
        .is_err()
    );
    input.text = "queued".into();
    let (record, queue) =
        input_queue::prepare_preview_checked(&mut db, id, request, 1, input_id, input, |db| {
            authorize(db, id)
        })
        .unwrap();
    db.execute_batch("create trigger fail_context_admission before insert on agent_input_queue_admissions begin select raise(abort,'fixture'); end;").unwrap();
    assert!(matches!(
        admit(
            &mut db,
            &record,
            Some(Confirmation {
                input_id,
                expected_revision: queue.revision()
            }),
            false
        ),
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        admit(
            &mut db,
            &record,
            Some(Confirmation {
                input_id,
                expected_revision: queue.revision() + 1
            }),
            false
        ),
        Err(Error::ConfirmationMismatch)
    ));
    let queue = input_queue::read_checked(&mut db, id, |db| authorize(db, id)).unwrap();
    assert_eq!(queue.inputs()[0].id, input_id);
    assert_eq!(queue.preparing(), Some(input_id));
}

#[test]
fn explicit_resume_admits_a_recovered_claim_without_resuming_a_later_pause_on_retry() {
    let (mut db, id, input_id, input) = fixture();
    let (record, queue) = input_queue::prepare_preview_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        1,
        input_id,
        input,
        |db| authorize(db, id),
    )
    .unwrap();
    let paused = input_queue::apply_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        queue.revision(),
        QueueEdit::Pause,
        |db| authorize(db, id),
    )
    .unwrap();
    let confirmation = Confirmation {
        input_id,
        expected_revision: paused.revision(),
    };
    admit(&mut db, &record, Some(confirmation), true).unwrap();
    let queue = input_queue::read_checked(&mut db, id, |db| authorize(db, id)).unwrap();
    assert!(!queue.paused());
    assert!(queue.inputs().is_empty());
    let paused = input_queue::apply_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        queue.revision(),
        QueueEdit::Pause,
        |db| authorize(db, id),
    )
    .unwrap();
    admit(&mut db, &record, Some(confirmation), true).unwrap();
    let final_queue = input_queue::read_checked(&mut db, id, |db| authorize(db, id)).unwrap();
    assert!(final_queue.paused());
    assert_eq!(final_queue.revision(), paused.revision());
}

#[test]
fn queued_mentions_resolve_to_frozen_sources_without_changing_the_original_queue_payload() {
    let (mut db, id, input_id, mut input) = fixture();
    let raw:Vec<research_agent_core::user_input::UserInput>=serde_json::from_value(serde_json::json!([
        {"type":"text","text":"queued","textElements":[]},
        {"type":"mention","name":"Note","target":{"kind":"file","sourceId":"context:note","resourceId":"note"}}
    ])).unwrap();
    input_queue::apply_checked(
        &mut db,
        id,
        Uuid::new_v4(),
        1,
        QueueEdit::Update {
            id: input_id,
            text: input.text.clone(),
            user_input: raw.clone(),
        },
        |db| authorize(db, id),
    )
    .unwrap();
    input.user_input = raw.clone();
    if let research_agent_core::user_input::UserInput::Mention {
        target: research_agent_core::user_input::MentionTarget::File { source_id, .. },
        ..
    } = &mut input.user_input[1]
    {
        *source_id = "resource:resolved".into();
    }
    let resolved = input.user_input.clone();
    let request = Uuid::new_v4();
    let tx = db.transaction().unwrap();
    assert!(
        input_queue::prepare_resolved_preview_in(&tx, id, request, 2, input_id, input.clone(), &[])
            .is_err()
    );
    let (record, queue) =
        input_queue::prepare_resolved_preview_in(&tx, id, request, 2, input_id, input, &raw)
            .unwrap();
    tx.commit().unwrap();
    assert_eq!(queue.inputs()[0].user_input, raw);
    assert_eq!(record.input.user_input, resolved);
    assert!(
        input_queue::apply_checked(
            &mut db,
            id,
            Uuid::new_v4(),
            queue.revision(),
            QueueEdit::Update {
                id: input_id,
                text: "changed".into(),
                user_input: vec![]
            },
            |db| authorize(db, id)
        )
        .is_err()
    );
    admit(
        &mut db,
        &record,
        Some(Confirmation {
            input_id,
            expected_revision: queue.revision(),
        }),
        false,
    )
    .unwrap();
    assert!(
        input_queue::read_checked(&mut db, id, |db| authorize(db, id))
            .unwrap()
            .inputs()
            .is_empty()
    );
}
