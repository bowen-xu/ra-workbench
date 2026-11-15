use super::*;
fn record() -> CheckpointRecord {
    CheckpointRecord {
        revision: 1,
        checkpoint: DocumentCheckpoint {
            schema_version: 1,
            manuscript_id: "synthetic".into(),
            relative_path: "main.tex".into(),
            version: 2,
            content: "original".into(),
            review_baseline: "baseline".into(),
            saved_sha256: "a".repeat(64),
            saved_version: 1,
            operations: vec![],
            save_intent: None,
            editor_receipt: None,
        },
    }
}
#[test]
fn native_read_cannot_hide_an_existing_checkpoint_or_replace_its_data_and_revision() {
    let record = record();
    let rows = DataRows {
        rows: vec![vec![
            DataValue::Integer(1),
            DataValue::Text(serde_json::to_string(&record.checkpoint).unwrap()),
        ]],
    };
    assert!(record_matches(&rows, Some(&record)));
    assert!(!record_matches(&rows, None));
    let mut changed = record.clone();
    changed.revision = 2;
    assert!(!record_matches(&rows, Some(&changed)));
    changed = record.clone();
    changed.checkpoint.content = "different".into();
    assert!(!record_matches(&rows, Some(&changed)));
    let absent = DataRows { rows: vec![] };
    assert!(record_matches(&absent, None));
    assert!(!record_matches(&absent, Some(&record)));
}
#[test]
fn core_storage_binding_preserves_idempotent_receipts_and_rejects_a_forged_cas_counter() {
    let current = record();
    let mut checkpoint = current.checkpoint.clone();
    checkpoint.version += 1;
    checkpoint.content.push_str(" input");
    let mut write = CheckpointWrite {
        expected_revision: Some(1),
        checkpoint: checkpoint.clone(),
    };
    let mut plan = CheckpointPlan {
        write: write.clone(),
        record: CheckpointRecord {
            revision: 2,
            checkpoint,
        },
        write_required: true,
    };
    assert!(plan_matches(&write, Some(&current), &plan));
    plan.record.revision = 3;
    assert!(!plan_matches(&write, Some(&current), &plan));
    plan.record.revision = 2;
    write.expected_revision = None;
    plan.write = write.clone();
    assert!(!plan_matches(&write, Some(&current), &plan));
    write.expected_revision = Some(1);
    plan.write = write.clone();
    plan.write_required = false;
    let receipt = plan.record.clone();
    assert!(plan_matches(&write, Some(&receipt), &plan));
    assert!(!plan_matches(&write, None, &plan));
}
