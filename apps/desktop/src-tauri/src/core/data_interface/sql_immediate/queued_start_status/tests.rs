//! Reuse the real typed preparation fixture; inspection never mutates it.
use super::*;
use crate::{
    agent::{
        execution_profile::AgentExecutionMode,
        repository::input_queue::start_inspection::{Fact, QueueStartIntent},
    },
    data_interface::{QueueStartInspection, journal::JournalCommand},
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, TaskKind, TurnOutcome},
    submission::SubmissionRecord,
};
use research_data_runtime::WriteOptions;

fn prepared() -> (Fixture, QueueStartIntent, SubmissionRecord) {
    prepared_with_model(ModelIdentity {
        provider_id: "deepseek".into(),
        model_id: "deepseek-v4-pro".into(),
        revision: None,
    })
}
fn prepared_with_model(model: ModelIdentity) -> (Fixture, QueueStartIntent, SubmissionRecord) {
    let f = Fixture::new();
    let mut source = f.source();
    source.model.identity = model;
    let prepared = f.db.prepare_queue_preview(&f.intent, source).unwrap();
    let record = prepared.record;
    let intent = QueueStartIntent {
        input_id: f.intent.input_id,
        expected_revision: prepared.queue.revision(),
        conversation_id: f.intent.conversation_id,
        manuscript_id: f.intent.manuscript_id,
        request_id: f.intent.request_id,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        mode: AgentExecutionMode::DeepSeekNonThinking,
    };
    (f, intent, record)
}
fn admitted(f: &Fixture, intent: &QueueStartIntent, record: &SubmissionRecord) {
    input_queue::admit_and_resume_checked(
        &mut f.db.lock().unwrap(),
        intent.conversation_id,
        intent.request_id,
        intent.expected_revision,
        intent.input_id,
        record.input.clone(),
        |_| Ok(()),
    )
    .unwrap();
}
fn cause(value: QueueStartInspection) -> &'static str {
    let QueueStartInspection::Unconfirmed(detail) = value else {
        panic!("inspection must be unconfirmed");
    };
    assert_eq!(detail.phase, "queue-start-status");
    detail.safe_cause
}
fn events(record: &SubmissionRecord, finish: bool) -> Vec<AgentEvent> {
    let mut payloads = vec![EventPayload::TurnStarted {
        turn_id: record.turn_id,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        task_kind: TaskKind::Regular,
    }];
    if finish {
        payloads.push(EventPayload::TurnFinished {
            turn_id: record.turn_id,
            outcome: TurnOutcome::Completed,
        });
    }
    payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(
                ConversationId::from_uuid(record.conversation_id),
                index as u64 + 1,
                0,
                record.request_id,
                payload,
            )
            .unwrap()
        })
        .collect()
}
fn append(f: &Fixture, record: &SubmissionRecord, finish: bool) {
    f.db.write(
        JournalCommand::new(
            ConversationId::from_uuid(record.conversation_id),
            Uuid::new_v4(),
            0,
            events(record, finish),
            None,
            1_790_000_000_000,
        )
        .unwrap(),
        WriteOptions::default(),
    )
    .unwrap();
}
fn changes(f: &Fixture) -> u64 {
    f.db.raw.lock().unwrap().total_changes()
}

#[test]
fn started_history_remains_observable_when_its_model_has_no_current_execution_profile() {
    let (f, intent, record) = prepared_with_model(ModelIdentity {
        provider_id: "deepseek".into(),
        model_id: "retired-historical-model".into(),
        revision: Some("historical-revision".into()),
    });
    admitted(&f, &intent, &record);
    // An unstarted record still cannot claim a currently executable mode.
    assert_eq!(
        cause(f.db.inspect_queue_start(&intent)),
        "queue_start_mode_mismatch"
    );
    append(&f, &record, true);
    f.db.materialize().unwrap();
    let before = changes(&f);
    assert!(matches!(f.db.inspect_queue_start(&intent),
        QueueStartInspection::Confirmed(Fact::Started {turn_id, outcome: Some(TurnOutcome::Completed)}) if turn_id == record.turn_id));
    assert_eq!(changes(&f), before);
}

#[test]
fn original_prepared_fact_survives_queue_drift_and_reads_no_mutable_materialization() {
    let (f, intent, record) = prepared();
    f.edit(QueueEdit::Add(QueuedInput {
        id: Uuid::new_v4(),
        text: "later tail".into(),
        user_input: vec![],
        origin: None,
    }));
    f.edit(QueueEdit::Pause);
    std::fs::remove_file(
        f.store
            .manuscript_directory()
            .join(intent.manuscript_id.to_string())
            .join("source/main.tex"),
    )
    .unwrap();
    let current = f.queue();
    let before = changes(&f);
    assert!(
        matches!(f.db.inspect_queue_start(&intent),QueueStartInspection::Confirmed(Fact::Prepared(saved)) if *saved==record)
    );
    assert_eq!(changes(&f), before);
    assert_eq!(f.queue(), current);
}

#[test]
fn exact_admitted_fact_does_not_resume_later_pause_and_never_returns_old_queue_snapshot() {
    let (f, intent, record) = prepared();
    admitted(&f, &intent, &record);
    f.edit(QueueEdit::Add(QueuedInput {
        id: Uuid::new_v4(),
        text: "retained tail".into(),
        user_input: vec![],
        origin: None,
    }));
    f.edit(QueueEdit::Pause);
    let current = f.queue();
    let before = changes(&f);
    assert!(
        matches!(f.db.inspect_queue_start(&intent),QueueStartInspection::Confirmed(Fact::Admitted(saved)) if *saved==record)
    );
    assert_eq!(changes(&f), before);
    assert_eq!(f.queue(), current);
}

#[test]
fn every_original_identity_and_mode_is_checked_before_confirming_admission() {
    let (f, intent, record) = prepared();
    admitted(&f, &intent, &record);
    for variant in 0..8 {
        let mut wrong = intent.clone();
        match variant {
            0 => wrong.input_id = Uuid::new_v4(),
            1 => wrong.expected_revision += 1,
            2 => wrong.request_id = Uuid::new_v4(),
            3 => wrong.manuscript_id = Uuid::new_v4(),
            4 => wrong.context_snapshot_id = Uuid::new_v4(),
            5 => wrong.model.revision = Some("other".into()),
            6 => wrong.conversation_id = Uuid::new_v4(),
            _ => wrong.mode = AgentExecutionMode::ChatGptDefault,
        }
        assert!(matches!(
            f.db.inspect_queue_start(&wrong),
            QueueStartInspection::Unconfirmed(_)
        ));
    }
}

#[test]
fn legacy_prepared_and_corrupt_admission_are_not_fabricated_execution_proofs() {
    let (f, intent, record) = prepared();
    f.db.raw
        .lock()
        .unwrap()
        .execute(
            "delete from data_sql_operation_receipts where operation_id=?1",
            [intent.request_id.to_string()],
        )
        .unwrap();
    assert_eq!(
        cause(f.db.inspect_queue_start(&intent)),
        "queue_start_legacy_preparation_unconfirmed"
    );
    // The deployed admission has its own exact C/I/R/Q and immutable record;
    // it does not require fabricating a typed start or mode receipt.
    admitted(&f, &intent, &record);
    assert!(matches!(
        f.db.inspect_queue_start(&intent),
        QueueStartInspection::Confirmed(Fact::Admitted(_))
    ));
    f.db.raw.lock().unwrap().execute("update agent_input_queue_admissions set result_json=json_set(result_json,'$.revision',999) where input_id=?1",[intent.input_id.to_string()]).unwrap();
    assert_eq!(
        cause(f.db.inspect_queue_start(&intent)),
        "queue_start_admission_snapshot_invalid"
    );
}

#[test]
fn pending_start_is_not_projected_or_misclassified_as_unstarted_or_missing() {
    let (f, intent, record) = prepared();
    admitted(&f, &intent, &record);
    append(&f, &record, true);
    let head = || {
        f.db.raw
            .lock()
            .unwrap()
            .query_row(
                "select materialized_position from data_runtime_state",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap()
    };
    let before = head();
    assert_eq!(
        cause(f.db.inspect_queue_start(&intent)),
        "queue_start_pending_history_unconfirmed"
    );
    assert_eq!(head(), before);
    f.db.materialize().unwrap();
    assert!(
        matches!(f.db.inspect_queue_start(&intent),QueueStartInspection::Confirmed(Fact::Started {turn_id,outcome:Some(TurnOutcome::Completed)}) if turn_id==record.turn_id)
    );
}

#[test]
fn closed_or_blocked_owner_keeps_sufficient_facts_but_cannot_prove_a_missing_start() {
    let (f, intent, record) = prepared();
    admitted(&f, &intent, &record);
    {
        let mut guard = f.db.owner.as_ref().unwrap().materialize_guard().unwrap();
        guard.block();
    }
    assert!(matches!(
        f.db.inspect_queue_start(&intent),
        QueueStartInspection::Confirmed(Fact::Admitted(_))
    ));
    let mut missing = intent.clone();
    missing.input_id = Uuid::new_v4();
    missing.request_id = Uuid::new_v4();
    assert_eq!(
        cause(f.db.inspect_queue_start(&missing)),
        "owner_blocked_receipt_unconfirmed"
    );
    let (f, intent, record) = prepared();
    admitted(&f, &intent, &record);
    append(&f, &record, true);
    f.db.materialize().unwrap();
    f.db.shutdown().unwrap();
    let before = changes(&f);
    assert!(matches!(
        f.db.inspect_queue_start(&intent),
        QueueStartInspection::Confirmed(Fact::Started {
            outcome: Some(TurnOutcome::Completed),
            ..
        })
    ));
    assert_eq!(changes(&f), before);
    let mut missing = intent.clone();
    missing.input_id = Uuid::new_v4();
    missing.request_id = Uuid::new_v4();
    assert_eq!(
        cause(f.db.inspect_queue_start(&missing)),
        "owner_closed_receipt_unconfirmed"
    );
}

#[test]
fn open_sql_transaction_does_not_commit_or_install_a_writer_during_status() {
    let (f, intent, _) = prepared();
    f.db.raw
        .lock()
        .unwrap()
        .execute_batch("begin deferred")
        .unwrap();
    assert_eq!(
        cause(f.db.inspect_queue_start(&intent)),
        "sql_transaction_open"
    );
    let db = f.db.raw.lock().unwrap();
    assert!(!db.is_autocommit());
    db.execute_batch("rollback").unwrap();
}

#[test]
fn sqlite_read_failures_retain_codes_and_original_fence_diagnostic_without_writes() {
    for fence_failure in [false, true] {
        let (f, intent, _) = prepared();
        f.db.raw
            .lock()
            .unwrap()
            .execute_batch(if fence_failure {
                "alter table data_runtime_state rename to unavailable_start_state"
            } else {
                "alter table agent_input_queue_admissions rename to unavailable_start_admissions"
            })
            .unwrap();
        let before = changes(&f);
        let QueueStartInspection::Unconfirmed(detail) = f.db.inspect_queue_start(&intent) else {
            panic!("failed SQL read must remain unconfirmed");
        };
        assert_eq!(detail.operation_id, intent.request_id);
        assert!(detail.epoch.is_some());
        assert_eq!(detail.sql_fence.as_deref(), Some("0"));
        assert_eq!(detail.sqlite_code.as_deref(), Some("Unknown"));
        assert_eq!(detail.sqlite_extended_code, Some(1));
        assert_eq!(
            detail.phase,
            if fence_failure {
                "status"
            } else {
                "queue-start-status"
            }
        );
        assert_eq!(
            detail.safe_cause,
            if fence_failure {
                "sql_registration_failed"
            } else {
                "queue_start_admission_unavailable"
            }
        );
        assert!(
            !serde_json::to_string(&detail)
                .unwrap()
                .contains("unavailable_start_")
        );
        assert_eq!(changes(&f), before);
    }
}

#[test]
fn missing_request_is_proved_only_without_any_original_queue_or_history_evidence() {
    let (f, mut intent, _) = prepared();
    intent.input_id = Uuid::new_v4();
    intent.request_id = Uuid::new_v4();
    assert!(matches!(
        f.db.inspect_queue_start(&intent),
        QueueStartInspection::Confirmed(Fact::Missing)
    ));
}

#[test]
fn prepared_proof_checks_real_fingerprint_and_the_original_full_queue_payload() {
    for fingerprint in [true, false] {
        let (f, intent, _) = prepared();
        let db = f.db.raw.lock().unwrap();
        if fingerprint {
            db.execute("update data_sql_operation_receipts set fingerprint=zeroblob(32) where operation_id=?1",[intent.request_id.to_string()]).unwrap();
        } else {
            let encoded: String = db
                .query_row(
                    "select snapshot_json from agent_input_queues where conversation_id=?1",
                    [intent.conversation_id.to_string()],
                    |row| row.get(0),
                )
                .unwrap();
            let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
            value["user_inputs"][0][1].as_array_mut().unwrap().push(json!({"type":"annotation","source":"source://paper/changed","title":"Changed reference","text":"Different evidence","comment":""}));
            db.execute(
                "update agent_input_queues set snapshot_json=?2 where conversation_id=?1",
                params![intent.conversation_id.to_string(), value.to_string()],
            )
            .unwrap();
        }
        drop(db);
        assert_eq!(
            cause(f.db.inspect_queue_start(&intent)),
            "queue_start_prepare_proof_invalid"
        );
    }
}

#[test]
fn journal_start_and_terminal_must_match_the_original_request_and_task_kind() {
    for variant in 0..4 {
        let (f, intent, record) = prepared();
        admitted(&f, &intent, &record);
        append(&f, &record, true);
        f.db.materialize().unwrap();
        let db = f.db.raw.lock().unwrap();
        let (ordinal, path, value) = match variant {
            0 => (0, "$.requestId", json!(Uuid::new_v4())),
            1 => (0, "$.payload.taskKind", json!("compact")),
            2 => (0, "$.payload.contextSnapshotId", json!(Uuid::new_v4())),
            _ => (1, "$.requestId", json!(Uuid::new_v4())),
        };
        db.execute("update agent_items set payload_json=json_set(payload_json,?3,json(?4)) where conversation_id=?1 and ordinal=?2",params![intent.conversation_id.to_string(),ordinal,path,value.to_string()]).unwrap();
        drop(db);
        assert!(matches!(
            f.db.inspect_queue_start(&intent),
            QueueStartInspection::Unconfirmed(_)
        ));
    }
}

#[test]
fn rollback_removes_visible_turn_but_does_not_reinterpret_executed_request_as_unstarted() {
    use research_agent_core::protocol::ItemKind;
    let (f, intent, record) = prepared();
    admitted(&f, &intent, &record);
    let item = Uuid::new_v4();
    let payloads = vec![
        EventPayload::TurnStarted {
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
            task_kind: TaskKind::Regular,
        },
        EventPayload::ItemStarted {
            turn_id: record.turn_id,
            item_id: item,
            kind: ItemKind::UserMessage,
        },
        EventPayload::TextDelta {
            turn_id: record.turn_id,
            item_id: item,
            text: record.input.text.clone(),
        },
        EventPayload::ItemCompleted {
            turn_id: record.turn_id,
            item_id: item,
        },
        EventPayload::TurnFinished {
            turn_id: record.turn_id,
            outcome: TurnOutcome::Completed,
        },
    ];
    let events = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(
                ConversationId::from_uuid(record.conversation_id),
                index as u64 + 1,
                0,
                record.request_id,
                payload,
            )
            .unwrap()
        })
        .collect();
    f.db.write(
        JournalCommand::new(
            ConversationId::from_uuid(record.conversation_id),
            Uuid::new_v4(),
            0,
            events,
            None,
            1_790_000_000_000,
        )
        .unwrap(),
        WriteOptions::default(),
    )
    .unwrap();
    f.db.materialize().unwrap();
    let event = AgentEvent::new(
        ConversationId::from_uuid(record.conversation_id),
        6,
        0,
        Uuid::new_v4(),
        EventPayload::ConversationRolledBack {
            turn_id: record.turn_id,
            user_turns: 1,
        },
    )
    .unwrap();
    // This persisted-history fixture isolates read semantics; production rollback
    // authorization and mutation have their own existing transaction tests.
    let db = f.db.raw.lock().unwrap();
    db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,5,1,?2)",params![record.conversation_id.to_string(),serde_json::to_string(&event).unwrap()]).unwrap();
    db.execute(
        "update agent_conversations set durable_count=6 where id=?1",
        [record.conversation_id.to_string()],
    )
    .unwrap();
    drop(db);
    assert!(
        matches!(f.db.inspect_queue_start(&intent),QueueStartInspection::Confirmed(Fact::Started {turn_id,outcome:Some(TurnOutcome::Completed)}) if turn_id==record.turn_id)
    );
}
