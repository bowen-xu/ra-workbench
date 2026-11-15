use super::*;
use crate::agent::repository::{create_conversation, submissions};
use research_agent_core::{
    conversation::ConversationId, protocol::ModelIdentity, submission::SubmissionInput,
};

#[test]
fn racing_confirmation_and_cancellation_have_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("race.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    let (scope, id, preview, input) = setup(&mut db);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other = barrier.clone();
    let cancel_path = path.clone();
    let cancel = std::thread::spawn(move || {
        let mut db = Connection::open(cancel_path).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        other.wait();
        cancel_preview_checked(&mut db, scope, Uuid::new_v4(), 2, id, preview, |_| Ok(())).is_ok()
    });
    db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    barrier.wait();
    let admitted = admit_checked(&mut db, scope, preview, 2, id, input, |_| Ok(())).is_ok();
    let canceled = cancel.join().unwrap();
    assert_ne!(admitted, canceled);
    let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
    assert_eq!(queue.inputs().is_empty(), admitted);
    assert_eq!(queue.paused(), canceled);
    assert_eq!(queue.preparing(), None);
}
fn setup(db: &mut Connection) -> (Uuid, Uuid, Uuid, SubmissionInput) {
    crate::storage_migration_tests::migrate_through(db, 94);
    let scope = Uuid::new_v4();
    let id = Uuid::new_v4();
    let preview = Uuid::new_v4();
    create_conversation(db, ConversationId::from_uuid(scope), "cancel fixture").unwrap();
    let input = SubmissionInput {
        remote_compaction_v2: None,
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        text: "retained input".into(),
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
    prepare_preview_checked(db, scope, preview, 1, id, input.clone(), |_| Ok(())).unwrap();
    (scope, id, preview, input)
}
#[test]
fn cancel_survives_reopen_retains_text_and_rejects_late_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cancel.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    let (scope, id, preview, input) = setup(&mut db);
    let request = Uuid::new_v4();
    let queue =
        cancel_preview_checked(&mut db, scope, request, 2, id, preview, |_| Ok(())).unwrap();
    assert!(queue.paused());
    assert_eq!(queue.preparing(), None);
    assert_eq!(queue.inputs()[0].text, input.text);
    assert!(submissions::read(&db, scope, preview).unwrap().is_some());
    drop(db);
    let mut db = Connection::open(path).unwrap();
    assert_eq!(
        cancel_preview_checked(&mut db, scope, request, 2, id, preview, |_| Ok(())).unwrap(),
        queue
    );
    assert!(admit_checked(&mut db, scope, preview, 2, id, input.clone(), |_| Ok(())).is_err());
    apply_checked(&mut db, scope, Uuid::new_v4(), 3, QueueEdit::Resume, |_| {
        Ok(())
    })
    .unwrap();
    let next = Uuid::new_v4();
    let (_, new_queue) =
        prepare_preview_checked(&mut db, scope, next, 4, id, input, |_| Ok(())).unwrap();
    // A replay of cancellation cannot release the new preview's head.
    cancel_preview_checked(&mut db, scope, request, 2, id, preview, |_| Ok(())).unwrap();
    assert_eq!(read_checked(&mut db, scope, |_| Ok(())).unwrap(), new_queue);
}
#[test]
fn failed_cancel_receipt_rolls_back_release_and_preview_removal() {
    let mut db = Connection::open_in_memory().unwrap();
    let (scope, id, preview, input) = setup(&mut db);
    db.execute_batch("create trigger fail_cancel before insert on agent_input_queue_receipts begin select raise(abort,'fixture'); end;").unwrap();
    assert!(
        cancel_preview_checked(&mut db, scope, Uuid::new_v4(), 2, id, preview, |_| Ok(())).is_err()
    );
    let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
    assert_eq!(queue.preparing(), Some(id));
    assert!(admit_checked(&mut db, scope, preview, 2, id, input, |_| Ok(())).is_ok());
}
#[test]
fn cancellation_cannot_undo_admission_or_bypass_current_authorization() {
    let mut db = Connection::open_in_memory().unwrap();
    let (scope, id, preview, input) = setup(&mut db);
    let request = Uuid::new_v4();
    assert!(matches!(
        cancel_preview_checked(&mut db, scope, request, 2, id, preview, |_| Err(
            Error::Closed
        )),
        Err(Error::Closed)
    ));
    admit_checked(&mut db, scope, preview, 2, id, input, |_| Ok(())).unwrap();
    assert!(cancel_preview_checked(&mut db, scope, request, 3, id, preview, |_| Ok(())).is_err());
    assert!(
        read_checked(&mut db, scope, |_| Ok(()))
            .unwrap()
            .inputs()
            .is_empty()
    );
}

#[test]
fn current_schema_ordinary_preview_cancel_keeps_its_exact_replay_contract() {
    let mut db = Connection::open_in_memory().unwrap();
    let (scope, id, preview, input) = setup(&mut db);
    crate::storage::migrate(&mut db).unwrap();
    let operation = Uuid::new_v4();
    let first =
        cancel_preview_checked(&mut db, scope, operation, 2, id, preview, |_| Ok(())).unwrap();
    assert_eq!(first.inputs()[0].text, input.text);
    assert!(first.paused());
    assert_eq!(first.preparing(), None);
    assert_eq!(
        cancel_preview_checked(&mut db, scope, operation, 2, id, preview, |_| Ok(())).unwrap(),
        first
    );
}

#[test]
fn ordinary_cancel_cannot_release_unstarted_or_started_typed_compact_claims() {
    use crate::{
        agent::writing_context::{WritingContextFile, WritingContextRequest},
        agent::{
            repository::input_queue::queued_compaction::{PrepareIntent, PrepareSource},
            writing_submission::{FrozenWritingModel, WritingSubmissionRequest},
        },
        data_interface::{Database, journal::JournalCommand},
        storage::AppStore,
    };
    use research_agent_core::protocol::{AgentEvent, EventPayload, TaskKind};
    use research_data_runtime::{WriteMode, WriteOptions};
    use sha2::{Digest, Sha256};
    for started in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("compact.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let scope = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        let id = Uuid::new_v4();
        let request = Uuid::new_v4();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic compact cancel')",
            [scope.to_string()],
        )
        .unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('project','Synthetic','','now')",[]).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'project','Synthetic','main.tex','now','now')",[manuscript.to_string()]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![scope.to_string(),manuscript.to_string()]).unwrap();
        let hash = format!("{:x}", Sha256::digest(b"Frozen manuscript"));
        sql.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'main.tex','tex',17,?2,'now')",params![manuscript.to_string(),hash]).unwrap();
        apply_checked(
            &mut sql,
            scope,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id,
                text: "/compact".into(),
                user_input: vec![],
                origin: None,
            }),
            |_| Ok(()),
        )
        .unwrap();
        let database = Database::open(sql, &root.path().join("data-log")).unwrap();
        let store = AppStore::for_test_database(database.clone(), root.path().join("objects"));
        let source = store
            .manuscript_directory()
            .join(manuscript.to_string())
            .join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("main.tex"), "Frozen manuscript").unwrap();
        let intent = PrepareIntent {
            conversation_id: scope,
            manuscript_id: manuscript,
            input_id: id,
            request_id: request,
            expected_queue_revision: 1,
            expected_revision: 0,
            previous_request_id: None,
            request_json: serde_json::json!({"inputId":id,"expectedQueueRevision":1,"previousRequestId":null,"expectedRevision":0,"submission":{"conversationId":scope,"requestId":request,"text":"/compact","context":{"manuscriptId":manuscript,"files":[{"relativePath":"main.tex","expectedDiskSha256":hash,"buffer":null,"selection":null}]},"model":null}}),
        };
        let prepared = database
            .prepare_queued_compaction(
                &intent,
                PrepareSource {
                    store,
                    request: WritingSubmissionRequest {
                        conversation_id: scope,
                        request_id: request,
                        text: "/compact".into(),
                        user_input: vec![],
                        context: WritingContextRequest {
                            manuscript_id: manuscript,
                            files: vec![WritingContextFile {
                                relative_path: "main.tex".into(),
                                expected_disk_sha256: hash,
                                buffer: None,
                                selection: None,
                            }],
                        },
                        resources: vec![],
                        retrieval: None,
                        model: None,
                        session_config: None,
                        turn_config: None,
                    },
                    model: FrozenWritingModel {
                        identity: ModelIdentity {
                            provider_id: "fixture".into(),
                            model_id: "synthetic".into(),
                            revision: None,
                        },
                        reasoning_effort: None,
                        service_tier: None,
                    },
                },
            )
            .unwrap();
        if started {
            let event = AgentEvent::new(
                ConversationId::from_uuid(scope),
                1,
                0,
                request,
                EventPayload::TurnStarted {
                    task_kind: TaskKind::Compact,
                    turn_id: prepared.record.turn_id,
                    context_snapshot_id: prepared.record.context_snapshot_id,
                    model: prepared.record.input.model.clone(),
                },
            )
            .unwrap();
            database
                .write(
                    JournalCommand::new(
                        ConversationId::from_uuid(scope),
                        Uuid::new_v4(),
                        0,
                        vec![event],
                        None,
                        1_790_000_000_000,
                    )
                    .unwrap(),
                    WriteOptions {
                        mode: WriteMode::Immediate,
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let mut sql = database.lock().unwrap();
        let before = read_checked(&mut sql, scope, |_| Ok(())).unwrap();
        let operation = Uuid::new_v4();
        assert!(matches!(
            cancel_preview_checked(
                &mut sql,
                scope,
                operation,
                before.revision(),
                id,
                request,
                |_| Ok(())
            ),
            Err(Error::Queue(QueueError::Conflict))
        ));
        assert_eq!(read_checked(&mut sql, scope, |_| Ok(())).unwrap(), before);
        assert_eq!(
            sql.query_row(
                "select request_id from agent_input_queue_previews where input_id=?1",
                [id.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            request.to_string()
        );
        assert_eq!(
            sql.query_row(
                "select count(*) from agent_input_queue_receipts where request_id=?1",
                [operation.to_string()],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            sql.query_row(
                "select count(*) from agent_input_queue_admissions where input_id=?1",
                [id.to_string()],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
    }
}
