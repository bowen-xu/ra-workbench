use super::*;

#[test]
fn exact_legacy_cancel_remains_unconfirmed_until_typed_write_replay_proves_its_scope() {
    let f = Fixture::new();
    let legacy = f.legacy_cancel();
    let original: String =
        f.db.lock()
            .unwrap()
            .query_row(
                "select command_json from agent_input_queue_receipts where request_id=?1",
                [f.intent.request_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(
        original,
        json!([
            1,
            2,
            [
                "cancelPreview",
                f.intent.input_id,
                f.intent.preview_request_id
            ]
        ])
        .to_string()
    );
    assert_eq!(f.count(&f.observer()), (1, 0));
    assert!(
        matches!(f.db.queue_preview_cancel_status(&f.intent).unwrap(),QueuePreviewCancelStatus::Unconfirmed(detail) if detail.safe_cause=="legacy_cancel_receipt_without_sql_proof")
    );
    assert_eq!(f.count(&f.observer()), (1, 0));
    let (_, newer) = f.next_preview();
    let (queue, proof) = f.db.cancel_queue_preview(&f.intent).unwrap();
    assert_eq!(queue, legacy);
    assert!(!proof.replayed);
    assert_eq!(f.queue(), newer);
    assert_eq!(f.count(&f.observer()), (1, 1));
    assert_eq!(
        f.db.lock()
            .unwrap()
            .query_row(
                "select command_json from agent_input_queue_receipts where request_id=?1",
                [f.intent.request_id.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        original
    );
    let f = f.reopen();
    assert!(
        matches!(f.db.queue_preview_cancel_status(&f.intent).unwrap(),QueuePreviewCancelStatus::Committed{queue,..} if queue==legacy)
    );
    assert_eq!(f.queue(), newer);
}

#[test]
fn legacy_receipt_cannot_be_rebound_to_another_live_manuscript_on_the_same_conversation() {
    let f = Fixture::new();
    f.legacy_cancel();
    let different = Uuid::new_v4();
    {
        let sql = f.db.lock().unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'project','Other synthetic','other.tex','now','now')",[different.to_string()]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)",params![f.intent.conversation_id.to_string(),different.to_string()]).unwrap();
    }
    let mut wrong = f.intent.clone();
    wrong.manuscript_id = different;
    assert!(matches!(
        f.db.queue_preview_cancel_status(&wrong),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert!(matches!(
        f.db.cancel_queue_preview(&wrong),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(f.count(&f.observer()), (1, 0));
    f.db.cancel_queue_preview(&f.intent).unwrap();
}

#[test]
fn other_business_family_and_other_typed_family_never_acquire_cancel_proof() {
    for typed in [false, true] {
        let f = Fixture::new();
        let before = f.queue();
        {
            let sql = f.db.lock().unwrap();
            if typed {
                sql.execute("insert into data_sql_operation_receipts(operation_id,epoch,kind,version,fingerprint,sql_fence_position,result_json) select ?1,epoch,'different.family',1,?2,materialized_position,'{}' from data_runtime_state",params![f.intent.request_id.to_string(),[7u8;32].as_slice()]).unwrap();
            } else {
                sql.execute("insert into agent_input_queue_receipts(request_id,conversation_id,command_json,result_json) values(?1,?2,?3,?4)",params![f.intent.request_id.to_string(),f.intent.conversation_id.to_string(),json!([1,2,["pause"]]).to_string(),json!({"wrong":"family"}).to_string()]).unwrap();
            }
        }
        assert!(f.db.queue_preview_cancel_status(&f.intent).is_err());
        assert!(f.db.cancel_queue_preview(&f.intent).is_err());
        assert_eq!(f.queue(), before);
        assert_eq!(f.count(&f.observer()), if typed { (0, 1) } else { (1, 0) });
    }
}

#[test]
fn journal_operation_collision_rejects_cancel_before_business_mutation() {
    let f = Fixture::new();
    let before = f.queue();
    {
        let sql = f.db.lock().unwrap();
        sql.execute("insert into data_operation_receipts(operation_id,epoch,position,fingerprint,result_json) select ?1,epoch,1,?2,'0' from data_runtime_state",params![f.intent.request_id.to_string(),[9u8;32].as_slice()]).unwrap();
    }
    assert!(matches!(
        f.db.queue_preview_cancel_status(&f.intent),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        f.db.cancel_queue_preview(&f.intent),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(f.queue(), before);
    assert_eq!(f.count(&f.observer()), (0, 0));
}

#[test]
fn current_binding_and_compact_purpose_are_checked_even_for_exact_cancel_replay() {
    let f = Fixture::new();
    f.db.cancel_queue_preview(&f.intent).unwrap();
    f.db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.intent.conversation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        f.db.cancel_queue_preview(&f.intent),
        Err(SqlImmediateFailure::Domain(CatalogError::NotFound))
    ));
    assert!(matches!(
        f.db.queue_preview_cancel_status(&f.intent).unwrap(),
        QueuePreviewCancelStatus::Committed { .. }
    ));
    let f = Fixture::new();
    let before = f.queue();
    {
        let sql = f.db.lock().unwrap();
        sql.execute("insert into agent_compaction_preparations(conversation_id,request_id,expected_revision) values(?1,?2,0)",params![f.intent.conversation_id.to_string(),f.intent.preview_request_id.to_string()]).unwrap();
    }
    assert!(matches!(
        f.db.cancel_queue_preview(&f.intent),
        Err(SqlImmediateFailure::Domain(CatalogError::Conflict))
    ));
    assert_eq!(f.queue(), before);
    assert_eq!(f.count(&f.observer()), (0, 0));
}

#[test]
fn a_lost_legacy_bridge_ack_never_reexecutes_or_advances_the_original_cancel() {
    for (fault, committed) in [(Fault::BeforeCommit, false), (Fault::AfterCommit, true)] {
        let f = Fixture::new();
        let legacy = f.legacy_cancel();
        assert!(matches!(
            f.db.cancel_queue_preview_owned(&f.intent, fault),
            Err(SqlImmediateFailure::Unknown(_))
        ));
        assert_eq!(
            f.count(&f.observer()),
            if committed { (1, 1) } else { (1, 0) }
        );
        let status = f.db.queue_preview_cancel_status(&f.intent).unwrap();
        assert_eq!(
            matches!(status, QueuePreviewCancelStatus::Committed { .. }),
            committed
        );
        assert!(f.db.cancel_queue_preview(&f.intent).is_err());
        let f = f.reopen();
        let (queue, _) = f.db.cancel_queue_preview(&f.intent).unwrap();
        assert_eq!(queue, legacy);
        assert_eq!(f.queue(), legacy);
        assert_eq!(f.count(&f.observer()), (1, 1));
    }
}
