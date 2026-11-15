use super::*;

#[test]
fn exact_legacy_replay_keeps_its_original_receipt_without_acquiring_a_host_proof() {
    let f = Fixture::new();
    let saved = f.legacy();
    let command: String = f
        .observer()
        .query_row(
            "select command_json from agent_input_queue_receipts where request_id=?1",
            [f.intent.request_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        matches!(f.db.input_queue_edit_status(&f.intent).unwrap(),QueueEditStatus::Unconfirmed(detail) if detail.safe_cause=="legacy_queue_edit_receipt_without_sql_proof")
    );
    let later = f.legacy_pause();
    assert!(
        matches!(f.db.edit_input_queue(&f.intent).unwrap(), QueueEditOutcome::LegacyReplay(queue) if queue==saved)
    );
    assert_eq!(f.queue(), later);
    assert_eq!(f.count(), (2, 0));
    assert_eq!(
        f.observer()
            .query_row(
                "select command_json from agent_input_queue_receipts where request_id=?1",
                [f.intent.request_id.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        command
    );
    let f = f.reopen();
    assert!(
        matches!(f.db.edit_input_queue(&f.intent).unwrap(),QueueEditOutcome::LegacyReplay(queue) if queue==saved)
    );
    assert_eq!(f.count(), (2, 0));
}

impl Fixture {
    fn legacy_pause(&self) -> InputQueue {
        input_queue::apply_checked(
            &mut self.db.lock().unwrap(),
            self.intent.conversation_id,
            Uuid::new_v4(),
            1,
            QueueEdit::Pause,
            |_| Ok(()),
        )
        .unwrap()
    }
}

#[test]
fn legacy_originless_add_is_compatible_but_readonly_status_rejects_another_command() {
    let mut f = Fixture::new();
    let saved = f.legacy();
    f.db.raw.lock().unwrap().execute("insert into research_projects(id,title,summary,created_at) values('p','Synthetic','','now')",[]).unwrap();
    let manuscript = Uuid::new_v4();
    f.db.raw.lock().unwrap().execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'p','Synthetic','main.tex','now','now')",[manuscript.to_string()]).unwrap();
    f.db.raw.lock().unwrap().execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![f.intent.conversation_id.to_string(),manuscript.to_string()]).unwrap();
    f.intent.scope = QueueEditScope::Manuscript {
        manuscript_id: manuscript,
    };
    assert!(matches!(
        f.db.input_queue_edit_status(&f.intent).unwrap(),
        QueueEditStatus::Unconfirmed(_)
    ));
    assert!(
        matches!(f.db.edit_input_queue(&f.intent).unwrap(),QueueEditOutcome::LegacyReplay(queue) if queue==saved)
    );
    let mut wrong = f.intent.clone();
    wrong.edit = QueueEdit::Pause;
    assert!(matches!(
        f.db.input_queue_edit_status(&wrong),
        Err(QueueEditFailure::Domain(input_queue::Error::Queue(
            QueueError::Conflict
        )))
    ));
    assert!(matches!(
        f.db.edit_input_queue(&wrong),
        Err(QueueEditFailure::Domain(input_queue::Error::Queue(
            QueueError::Conflict
        )))
    ));
    assert_eq!(f.count(), (1, 0));
}

#[test]
fn confirmed_domain_refusals_preserve_queue_error_and_rollback_every_business_row() {
    let f = Fixture::new();
    let mut bad = f.intent.clone();
    if let QueueEdit::Add(input) = &mut bad.edit {
        input.text.clear();
        input.user_input.clear();
    }
    assert!(matches!(
        f.db.edit_input_queue(&bad),
        Err(QueueEditFailure::Domain(input_queue::Error::Queue(
            QueueError::Invalid
        )))
    ));
    assert_eq!(f.count(), (0, 0));
    assert_eq!(
        f.observer()
            .query_row(
                "select count(*) from agent_input_queue_identities",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    f.db.edit_input_queue(&f.intent).unwrap();
    assert!(matches!(
        f.db.edit_input_queue(&f.next(0, QueueEdit::Pause)),
        Err(QueueEditFailure::Domain(input_queue::Error::Queue(
            QueueError::Conflict
        )))
    ));
    f.db.raw
        .lock()
        .unwrap()
        .execute(
            "update agent_conversations set archived=1 where id=?1",
            [f.intent.conversation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        f.db.edit_input_queue(&f.intent),
        Err(QueueEditFailure::Domain(input_queue::Error::Closed))
    ));
    assert!(matches!(
        f.db.input_queue_edit_status(&f.intent).unwrap(),
        QueueEditStatus::Committed { .. }
    ));
    assert_eq!(f.count(), (1, 1));
}

#[test]
fn no_unmanaged_database_or_internal_execution_edit_can_silently_use_legacy_writes() {
    let f = Fixture::new();
    let mut sql = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    use crate::data_interface::IntoDatabase;
    let unmanaged = std::sync::Arc::new(std::sync::Mutex::new(sql)).into_database();
    assert!(matches!(
        unmanaged.edit_input_queue(&f.intent),
        Err(QueueEditFailure::Storage(SqlImmediateFailure::Unmanaged(_)))
    ));
    for edit in [
        QueueEdit::Prepare(Uuid::new_v4()),
        QueueEdit::Release(Uuid::new_v4()),
        QueueEdit::Admitted(Uuid::new_v4()),
    ] {
        assert!(matches!(
            f.db.edit_input_queue(&f.next(0, edit)),
            Err(QueueEditFailure::Storage(SqlImmediateFailure::Invalid(_)))
        ));
    }
    assert_eq!(f.count(), (0, 0));
}

#[test]
fn typed_busy_and_capacity_refusals_keep_the_original_queue_error_variants() {
    let f = Fixture::new();
    let (queue, _) = committed(f.db.edit_input_queue(&f.intent).unwrap());
    let input = queue.inputs()[0].id;
    input_queue::apply_checked(
        &mut f.db.lock().unwrap(),
        f.intent.conversation_id,
        Uuid::new_v4(),
        1,
        QueueEdit::Prepare(input),
        |_| Ok(()),
    )
    .unwrap();
    assert!(matches!(
        f.db.edit_input_queue(&f.next(
            2,
            QueueEdit::Update {
                id: input,
                text: "new".into(),
                user_input: vec![]
            }
        )),
        Err(QueueEditFailure::Domain(input_queue::Error::Queue(
            QueueError::Busy
        )))
    ));
    assert_eq!(f.count(), (2, 1));
    let f = Fixture::new();
    for revision in 0..64 {
        input_queue::apply_checked(
            &mut f.db.lock().unwrap(),
            f.intent.conversation_id,
            Uuid::new_v4(),
            revision,
            QueueEdit::Add(QueuedInput {
                id: Uuid::new_v4(),
                text: "synthetic capacity".into(),
                user_input: vec![],
                origin: None,
            }),
            |_| Ok(()),
        )
        .unwrap();
    }
    let mut full = f.intent.clone();
    full.expected_revision = 64;
    assert!(matches!(
        f.db.edit_input_queue(&full),
        Err(QueueEditFailure::Domain(input_queue::Error::Queue(
            QueueError::Capacity
        )))
    ));
    assert_eq!(f.count(), (64, 0));
    assert_eq!(f.queue().inputs().len(), 64);
}
