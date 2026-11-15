//! Real ordinary/manual preparation records and the immutable production journal.
use super::*;
use crate::{
    agent::repository::writing_start_inspection::{Fact, Intent},
    data_interface::{WritingStartInspection, journal::JournalCommand},
};
use research_agent_core::{
    conversation::ConversationId,
    protocol::{AgentEvent, EventPayload, ItemKind, TaskKind, TurnOutcome},
};
use research_data_runtime::WriteOptions;

fn intent(f: &Fixture, record: &SubmissionRecord) -> Intent {
    Intent {
        conversation_id: record.conversation_id,
        manuscript_id: f.intent.manuscript(),
        request_id: record.request_id,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        compact_revision: f.intent.compact_revision,
    }
}
fn cause(value: WritingStartInspection) -> &'static str {
    let WritingStartInspection::Unconfirmed(detail) = value else {
        panic!("uncertain observation required");
    };
    assert_eq!(detail.phase, "writing-start-status");
    detail.safe_cause
}
fn append(f: &Fixture, record: &SubmissionRecord, kind: TaskKind, user: bool, pending: bool) {
    let count: u64 =
        f.db.raw
            .lock()
            .unwrap()
            .query_row(
                "select durable_count from agent_conversations where id=?1",
                [record.conversation_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
    let mut payloads = vec![EventPayload::TurnStarted {
        turn_id: record.turn_id,
        context_snapshot_id: record.context_snapshot_id,
        model: record.input.model.clone(),
        task_kind: kind,
    }];
    if user {
        let item_id = Uuid::new_v4();
        payloads.extend([
            EventPayload::ItemStarted {
                turn_id: record.turn_id,
                item_id,
                kind: ItemKind::UserMessage,
            },
            EventPayload::TextDelta {
                turn_id: record.turn_id,
                item_id,
                text: record.input.text.clone(),
            },
            EventPayload::ItemCompleted {
                turn_id: record.turn_id,
                item_id,
            },
        ]);
    }
    payloads.push(EventPayload::TurnFinished {
        turn_id: record.turn_id,
        outcome: TurnOutcome::Completed,
    });
    let events = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(
                ConversationId::from_uuid(record.conversation_id),
                count + index as u64 + 1,
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
            count,
            events,
            None,
            1_790_000_000_000,
        )
        .unwrap(),
        WriteOptions::default(),
    )
    .unwrap();
    if !pending {
        f.db.materialize().unwrap();
    }
}
fn changes(f: &Fixture) -> u64 {
    f.db.raw.lock().unwrap().total_changes()
}

#[test]
fn ordinary_and_manual_are_readonly_prepared_then_started_without_current_model_or_source() {
    for compact in [false, true] {
        let f = Fixture::new(compact);
        let mut source = f.source();
        source.model.identity.model_id = "retired-historical-model".into();
        let record = f.db.prepare_writing(&f.intent, source).unwrap();
        let request = intent(&f, &record);
        std::fs::remove_file(
            f.store
                .manuscript_directory()
                .join(request.manuscript_id.to_string())
                .join("source/main.tex"),
        )
        .unwrap();
        let before = changes(&f);
        assert!(
            matches!(f.db.inspect_writing_start(&request), WritingStartInspection::Confirmed(Fact::Prepared(saved)) if *saved==record)
        );
        assert_eq!(changes(&f), before);
        append(
            &f,
            &record,
            if compact {
                TaskKind::Compact
            } else {
                TaskKind::Regular
            },
            false,
            false,
        );
        let before = changes(&f);
        assert!(
            matches!(f.db.inspect_writing_start(&request), WritingStartInspection::Confirmed(Fact::Started {turn_id,outcome:Some(TurnOutcome::Completed)}) if turn_id==record.turn_id)
        );
        assert_eq!(changes(&f), before);
    }
}

#[test]
fn pending_prefix_is_never_materialized_or_mistaken_for_an_unstarted_request() {
    for compact in [false, true] {
        let f = Fixture::new(compact);
        let record = f.prepare().unwrap();
        let request = intent(&f, &record);
        append(
            &f,
            &record,
            if compact {
                TaskKind::Compact
            } else {
                TaskKind::Regular
            },
            false,
            true,
        );
        let before = changes(&f);
        assert_eq!(
            cause(f.db.inspect_writing_start(&request)),
            "writing_start_pending_history_unconfirmed"
        );
        assert_eq!(changes(&f), before);
        f.db.materialize().unwrap();
        assert!(matches!(
            f.db.inspect_writing_start(&request),
            WritingStartInspection::Confirmed(Fact::Started { .. })
        ));
    }
}

#[test]
fn exact_scope_purpose_header_and_queue_origin_are_required() {
    for compact in [false, true] {
        let f = Fixture::new(compact);
        let record = f.prepare().unwrap();
        let request = intent(&f, &record);
        for field in 0..4 {
            let mut wrong = request.clone();
            match field {
                0 => wrong.manuscript_id = Uuid::new_v4(),
                1 => wrong.context_snapshot_id = Uuid::new_v4(),
                2 => wrong.model.revision = Some("different".into()),
                _ => wrong.compact_revision = if compact { Some(1) } else { Some(0) },
            };
            assert!(matches!(
                f.db.inspect_writing_start(&wrong),
                WritingStartInspection::Unconfirmed(_)
            ));
        }
        let db = f.db.raw.lock().unwrap();
        let input = Uuid::new_v4();
        db.execute(
            "insert into agent_input_queue_identities(input_id,conversation_id) values(?1,?2)",
            params![input.to_string(), request.conversation_id.to_string()],
        )
        .unwrap();
        db.execute("insert into agent_queue_submission_origins(conversation_id,request_id,input_id) values(?1,?2,?3)",
            params![request.conversation_id.to_string(),request.request_id.to_string(),input.to_string()]).unwrap();
        drop(db);
        assert_eq!(
            cause(f.db.inspect_writing_start(&request)),
            "writing_start_queue_origin_mismatch"
        );
    }
}

#[test]
fn supplemental_only_preparation_never_claims_a_historical_writing_host() {
    for compact in [false, true] {
        let mut f = Fixture::new(compact);
        let note = Uuid::new_v4();
        f.db.raw.lock().unwrap().execute(
            "insert into note_pages(id,title,created_at,updated_at) values(?1,'Supplemental only','now','now')",
            [note.to_string()],
        ).unwrap();
        f.intent.request.context.files.clear();
        f.intent.request.retrieval = None;
        f.intent.request.resources =
            vec![crate::agent::context_session::sources::Resource::Note { resource_id: note }];
        let record = f.prepare().unwrap();
        assert!(!record.input.context.is_empty());
        assert!(
            record.input.context.iter().all(|source| {
                serde_json::from_str::<serde_json::Value>(source.anchor.as_deref().unwrap())
                    .unwrap()["supplementalContext"]
                    == true
            })
        );
        let original = intent(&f, &record);
        for started in [false, true] {
            if started {
                append(
                    &f,
                    &record,
                    if compact {
                        TaskKind::Compact
                    } else {
                        TaskKind::Regular
                    },
                    false,
                    false,
                );
            }
            for manuscript in [original.manuscript_id, Uuid::new_v4()] {
                let mut request = original.clone();
                request.manuscript_id = manuscript;
                let before = changes(&f);
                assert_eq!(
                    cause(f.db.inspect_writing_start(&request)),
                    "writing_start_scope_unconfirmed"
                );
                assert_eq!(changes(&f), before);
            }
        }
    }
}

#[test]
fn frozen_main_scope_keeps_original_start_observable_after_current_binding_is_removed() {
    for compact in [false, true] {
        let f = Fixture::new(compact);
        let record = f.prepare().unwrap();
        let request = intent(&f, &record);
        append(
            &f,
            &record,
            if compact {
                TaskKind::Compact
            } else {
                TaskKind::Regular
            },
            false,
            false,
        );
        f.db.raw
            .lock()
            .unwrap()
            .execute(
                "delete from agent_conversation_bindings where conversation_id=?1",
                [record.conversation_id.to_string()],
            )
            .unwrap();
        let before = changes(&f);
        assert!(matches!(f.db.inspect_writing_start(&request),
            WritingStartInspection::Confirmed(Fact::Started {turn_id, outcome: Some(TurnOutcome::Completed)}) if turn_id == record.turn_id));
        assert_eq!(changes(&f), before);
    }
}

#[test]
fn closed_blocked_and_open_transaction_preserve_facts_without_claiming_absence() {
    for closed in [false, true] {
        let f = Fixture::new(false);
        let record = f.prepare().unwrap();
        let request = intent(&f, &record);
        append(&f, &record, TaskKind::Regular, false, false);
        if closed {
            f.db.shutdown().unwrap();
        } else {
            f.db.owner
                .as_ref()
                .unwrap()
                .materialize_guard()
                .unwrap()
                .block();
        }
        let before = changes(&f);
        assert!(matches!(
            f.db.inspect_writing_start(&request),
            WritingStartInspection::Confirmed(Fact::Started { .. })
        ));
        let mut missing = request.clone();
        missing.request_id = Uuid::new_v4();
        assert_eq!(
            cause(f.db.inspect_writing_start(&missing)),
            if closed {
                "owner_closed_receipt_unconfirmed"
            } else {
                "owner_blocked_receipt_unconfirmed"
            }
        );
        assert_eq!(changes(&f), before);
    }
    let f = Fixture::new(false);
    let record = f.prepare().unwrap();
    let request = intent(&f, &record);
    f.db.raw
        .lock()
        .unwrap()
        .execute_batch("begin deferred")
        .unwrap();
    assert_eq!(
        cause(f.db.inspect_writing_start(&request)),
        "sql_transaction_open"
    );
    let db = f.db.raw.lock().unwrap();
    assert!(!db.is_autocommit());
    db.execute_batch("rollback").unwrap();
}

#[test]
fn missing_result_with_original_proof_is_uncertain_and_sql_errors_keep_codes() {
    let f = Fixture::new(false);
    let record = f.prepare().unwrap();
    let request = intent(&f, &record);
    let mut missing = request.clone();
    missing.request_id = Uuid::new_v4();
    assert!(matches!(
        f.db.inspect_writing_start(&missing),
        WritingStartInspection::Confirmed(Fact::Missing)
    ));
    let db = f.db.raw.lock().unwrap();
    db.execute(
        "delete from agent_submissions where conversation_id=?1 and request_id=?2",
        params![
            request.conversation_id.to_string(),
            request.request_id.to_string()
        ],
    )
    .unwrap();
    drop(db);
    let before = changes(&f);
    assert_eq!(
        cause(f.db.inspect_writing_start(&request)),
        "writing_start_submission_missing"
    );
    assert_eq!(changes(&f), before);
    f.db.raw
        .lock()
        .unwrap()
        .execute_batch("alter table agent_submissions rename to unavailable_start_submissions")
        .unwrap();
    let WritingStartInspection::Unconfirmed(detail) = f.db.inspect_writing_start(&request) else {
        panic!()
    };
    assert_eq!(detail.sqlite_extended_code, Some(1));
    assert_eq!(detail.safe_cause, "writing_start_submission_unavailable");
    assert!(
        !serde_json::to_string(&detail)
            .unwrap()
            .contains("unavailable_start_submissions")
    );
}

#[test]
fn rollback_hides_ordinary_and_manual_turns_but_never_erases_original_admission() {
    for compact in [false, true] {
        let mut f = Fixture::new(compact);
        if compact {
            let mut prior = f.intent.clone();
            prior.request.request_id = Uuid::new_v4();
            prior.request.text = "Earlier regular input".into();
            prior.compact_revision = None;
            prior.request.retrieval = None;
            let mut source = f.source();
            source.request = prior.request.clone().into();
            let record = f.db.prepare_writing(&prior, source).unwrap();
            append(&f, &record, TaskKind::Regular, true, false);
            f.intent.compact_revision = Some(5);
        }
        let record = f.prepare().unwrap();
        let request = intent(&f, &record);
        append(
            &f,
            &record,
            if compact {
                TaskKind::Compact
            } else {
                TaskKind::Regular
            },
            !compact,
            false,
        );
        let db = f.db.raw.lock().unwrap();
        let count: u64 = db
            .query_row(
                "select durable_count from agent_conversations where id=?1",
                [request.conversation_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let event = AgentEvent::new(
            ConversationId::from_uuid(request.conversation_id),
            count + 1,
            0,
            Uuid::new_v4(),
            EventPayload::ConversationRolledBack {
                turn_id: record.turn_id,
                user_turns: 1,
            },
        )
        .unwrap();
        db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,?2,1,?3)",params![request.conversation_id.to_string(),count,serde_json::to_string(&event).unwrap()]).unwrap();
        db.execute(
            "update agent_conversations set durable_count=?2 where id=?1",
            params![request.conversation_id.to_string(), count + 1],
        )
        .unwrap();
        drop(db);
        assert!(
            matches!(f.db.inspect_writing_start(&request),WritingStartInspection::Confirmed(Fact::Started {turn_id,outcome:Some(TurnOutcome::Completed)}) if turn_id==record.turn_id)
        );
    }
}
