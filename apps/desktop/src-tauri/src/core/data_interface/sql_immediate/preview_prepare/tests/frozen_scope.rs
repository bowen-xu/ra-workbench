use super::*;

#[test]
fn empty_materialized_context_is_invalid_and_does_not_claim_or_block_the_queue() {
    let mut f = Fixture::new();
    f.intent.request_json["submission"]["context"]["files"] = json!([]);
    let before = f.queue();
    let error =
        f.db.prepare_queue_preview(&f.intent, f.source())
            .unwrap_err();
    assert!(matches!(error, SqlImmediateFailure::Invalid(_)));
    let detail = error.diagnostic().unwrap();
    assert_eq!(detail.safe_cause, "writing_context_invalid");
    assert_eq!(detail.phase, "queue-prepare-context");
    assert_eq!(detail.operation_id, f.intent.request_id);
    assert_eq!(f.queue(), before);
    assert_eq!(f.counts(&f.observer()), (0, 0, 0));
    assert!(f.db.lock().is_ok());
    assert!(matches!(
        f.db.read_queue_preview_prepare_status(&f.intent).unwrap(),
        QueuePreviewPrepareStatus::NotFound
    ));
}

#[test]
fn frozen_record_scope_tampering_never_becomes_a_committed_preparation() {
    for empty in [false, true] {
        let f = Fixture::new();
        let prepared = f.prepare();
        let mut input = prepared.record.input;
        if empty {
            input.context.clear();
        } else {
            let source = &mut input.context[0];
            let mut anchor: serde_json::Value =
                serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
            anchor["manuscriptId"] = json!(Uuid::new_v4());
            source.anchor = Some(anchor.to_string());
        }
        f.db.raw.lock().unwrap().execute(
            "update agent_submissions set input_json=?3 where conversation_id=?1 and request_id=?2",
            params![f.intent.conversation_id.to_string(), f.intent.request_id.to_string(), serde_json::to_string(&input).unwrap()],
        ).unwrap();
        assert!(matches!(
            f.db.read_queue_preview_prepare_status(&f.intent),
            Err(SqlImmediateFailure::Corrupt(_))
        ));
        // Inspection detects bad evidence without repairing or blocking an owner.
        assert!(f.db.lock().is_ok());
        assert!(matches!(
            f.db.replay_queue_preview_prepare(&f.intent),
            Err(SqlImmediateFailure::Corrupt(_))
        ));
        assert!(f.db.lock().is_err());
        assert_eq!(f.counts(&f.observer()), (1, 1, 1));
    }
}
