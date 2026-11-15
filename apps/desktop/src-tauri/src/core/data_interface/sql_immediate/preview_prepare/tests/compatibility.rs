use super::*;

#[test]
fn full_request_and_every_identity_field_are_frozen_by_the_typed_fingerprint() {
    let f = Fixture::new();
    f.prepare();
    let before = f.queue();
    for variant in 0..10 {
        let mut changed = f.intent.clone();
        match variant {
            0 => changed.conversation_id = Uuid::new_v4(),
            1 => changed.manuscript_id = Uuid::new_v4(),
            2 => changed.input_id = Uuid::new_v4(),
            3 => changed.expected_queue_revision += 1,
            4 => {
                changed.request_json["submission"]["resources"] =
                    json!([{"kind":"note","resourceId":Uuid::new_v4()}])
            }
            5 => changed.request_json["submission"]["userInput"] = json!([]),
            6 => {
                changed.request_json["submission"]["context"]["files"][0]["buffer"] =
                    json!("different buffer")
            }
            7 => {
                changed.request_json["submission"]["model"] =
                    json!({"providerId":"other","modelId":"different"})
            }
            8 => changed.request_json["submission"]["retrieval"] = json!({"different":"intent"}),
            _ => changed.request_json["submission"]["turnConfig"] = json!({"maxBytes":1024}),
        }
        assert!(matches!(
            f.db.read_queue_preview_prepare_status(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert!(matches!(
            f.db.replay_queue_preview_prepare(&changed),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert!(matches!(
            f.db.prepare_queue_preview(&changed, f.source()),
            Err(SqlImmediateFailure::Conflict(_))
        ));
        assert_eq!(f.queue(), before);
    }
}

#[test]
fn nested_object_key_order_does_not_change_the_canonical_original_intent() {
    let f = Fixture::new();
    let prepared = f.prepare();
    let mut equivalent = f.intent.clone();
    let root = equivalent.request_json.as_object_mut().unwrap();
    let revision = root.remove("expectedRevision").unwrap();
    root.insert("expectedRevision".into(), revision);
    let submission = root.get_mut("submission").unwrap().as_object_mut().unwrap();
    let conversation = submission.remove("conversationId").unwrap();
    submission.insert("conversationId".into(), conversation);
    assert_eq!(
        identity(&f.intent).unwrap().fingerprint,
        identity(&equivalent).unwrap().fingerprint
    );
    assert_eq!(
        f.db.replay_queue_preview_prepare(&equivalent)
            .unwrap()
            .unwrap()
            .record,
        prepared.record
    );
}

#[test]
fn legacy_preview_without_full_hash_stays_readable_but_never_acquires_a_new_prepare_proof() {
    let f = Fixture::new();
    let source = f.source();
    let mut sql = f.db.lock().unwrap();
    let tx = sql.transaction().unwrap();
    let original = source.request.user_input.clone();
    let input = crate::agent::writing_submission::materialize_submission_in(
        &tx,
        &source.store,
        source.request,
        source.model,
    )
    .unwrap();
    let (record, queue) = input_queue::prepare_resolved_preview_in(
        &tx,
        f.intent.conversation_id,
        f.intent.request_id,
        1,
        f.intent.input_id,
        input,
        &original,
    )
    .unwrap();
    tx.commit().unwrap();
    drop(sql);
    assert!(
        matches!(f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),QueuePreviewPrepareStatus::Unconfirmed(detail) if detail.safe_cause=="legacy_prepare_without_intent_proof")
    );
    assert!(matches!(
        f.db.replay_queue_preview_prepare(&f.intent),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(matches!(
        f.db.prepare_queue_preview(&f.intent, f.source()),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(f.counts(&f.observer()), (1, 0, 0));
    assert_eq!(f.queue(), queue);
    let pending = input_queue::read_pending_preview_checked(
        &mut f.db.lock().unwrap(),
        f.intent.conversation_id,
        |_| Ok(()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(pending.0, record);
}

#[test]
fn business_sql_and_journal_family_collisions_are_rejected_before_mutation() {
    for variant in 0..3 {
        let f = Fixture::new();
        let before = f.queue();
        {
            let sql = f.db.lock().unwrap();
            match variant {
                0 => {
                    sql.execute("insert into agent_input_queue_receipts(request_id,conversation_id,command_json,result_json) values(?1,?2,'[1,1,[\"pause\"]]','{}')",params![f.intent.request_id.to_string(),f.intent.conversation_id.to_string()]).unwrap();
                }
                1 => {
                    sql.execute("insert into data_sql_operation_receipts(operation_id,epoch,kind,version,fingerprint,sql_fence_position,result_json) select ?1,epoch,'other.family',1,?2,materialized_position,'{}' from data_runtime_state",params![f.intent.request_id.to_string(),[7u8;32].as_slice()]).unwrap();
                }
                _ => {
                    sql.execute("insert into data_operation_receipts(operation_id,epoch,position,fingerprint,result_json) select ?1,epoch,1,?2,'0' from data_runtime_state",params![f.intent.request_id.to_string(),[9u8;32].as_slice()]).unwrap();
                }
            }
        }
        assert!(f.db.read_queue_preview_prepare_status(&f.intent).is_err());
        assert!(f.db.replay_queue_preview_prepare(&f.intent).is_err());
        assert!(f.db.prepare_queue_preview(&f.intent, f.source()).is_err());
        assert_eq!(f.queue(), before);
        assert_eq!(f.counts(&f.observer()).0, 0);
    }
}

#[test]
fn write_replay_rechecks_live_binding_while_status_only_inspects_frozen_proof() {
    let f = Fixture::new();
    let original = f.prepare();
    f.db.lock()
        .unwrap()
        .execute(
            "delete from agent_conversation_bindings where conversation_id=?1",
            [f.intent.conversation_id.to_string()],
        )
        .unwrap();
    assert!(matches!(
        f.db.replay_queue_preview_prepare(&f.intent),
        Err(SqlImmediateFailure::Domain(CatalogError::NotFound))
    ));
    assert!(matches!(
        f.db.prepare_queue_preview(&f.intent, f.source()),
        Err(SqlImmediateFailure::Domain(CatalogError::NotFound))
    ));
    assert!(
        matches!(f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),QueuePreviewPrepareStatus::Committed {state,..} if state.record==original.record&&state.queue==original.queue)
    );
}
