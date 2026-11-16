use super::*;
use research_agent_core::submission::ContextSourceKind;
use rusqlite::{Connection, params};
use std::fs;

#[path = "fixture.rs"]
mod fixture;
use fixture::*;

#[test]
fn fixed_package_resources_keep_original_hashes_without_added_newline() {
    for (name, expected_hash) in [
        ("read", READ_HASH),
        ("expire", EXPIRE_HASH),
        ("mark-sent", MARK_HASH),
    ] {
        let bytes = fs::read(resource_directory().join(format!("trace-{name}.v1.sql"))).unwrap();
        assert!(!bytes.ends_with(b"\n"));
        assert_eq!(hash(std::str::from_utf8(&bytes).unwrap()), expected_hash);
    }
    let package = PackageFixture::new();
    let resources = package.resources();
    assert_eq!(resources.read.package_key(), resources.expire.package_key());
    assert_eq!(
        resources.read.package_key(),
        resources.mark_sent.package_key()
    );
}

#[test]
fn original_page_source_and_mark_share_callers_commit_and_survive_reopen() {
    let fixture = DatabaseFixture::new();
    fixture.insert_trace("Synthetic retained context", FUTURE);
    let package = PackageFixture::new();
    let page_id;
    {
        let mut connection = fixture.db.lock().unwrap();
        let tx = immediate(&mut connection);
        page_id = core_note(&tx, "Source page").unwrap();
        tx.commit().unwrap();
    }
    let prepared = fixture.capture(&package, &page_id);
    let mut input = prepared.note_input().unwrap();
    assert_eq!(input.query, QUERY);
    assert_eq!(input.row.as_ref().unwrap().paper_id, None);
    // Exported values cannot replace Core's captured read or source text.
    input.query = "host-side clone mutation".into();
    input.row.as_mut().unwrap().context_text = "host-side clone mutation".into();
    {
        let mut connection = fixture.db.lock().unwrap();
        let tx = immediate(&mut connection);
        let source = consume_prepared_trace_in(&tx, &prepared, &ready()).unwrap();
        assert_eq!(source.kind, ContextSourceKind::Retrieval);
        assert_eq!(source.source_id, format!("retrieval:{TRACE_ID}"));
        assert_eq!(source.resource_id, page_id);
        assert_eq!(source.revision, format!("trace:{TRACE_ID}"));
        assert_eq!(source.text, "Synthetic retained context");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(source.anchor.as_ref().unwrap()).unwrap(),
            serde_json::json!({"pageId":page_id,"traceId":TRACE_ID,"query":QUERY})
        );
        assert_eq!(status(&tx), "sent");
        let observer = Connection::open(fixture.directory.path().join("fixture.sqlite3")).unwrap();
        assert_eq!(status(&observer), "previewed");
        tx.commit().unwrap();
    }
    let fixture = fixture.reopen();
    assert_eq!(fixture.status(), "sent");
}

#[test]
fn sent_expiry_deleted_row_and_absence_phantom_change_the_full_readset() {
    for change in ["sent", "expired", "deleted", "inserted"] {
        let fixture = DatabaseFixture::new();
        if change != "inserted" {
            fixture.insert_trace("Original synthetic context", FUTURE);
        }
        let prepared = fixture.capture(&PackageFixture::new(), "page");
        if change == "inserted" {
            assert!(prepared.note_input().unwrap().row.is_none());
            fixture.insert_trace("A previously absent trace", FUTURE);
        } else {
            let connection = fixture.db.lock().unwrap();
            match change {
                "sent" => connection.execute(
                    "update concept_retrieval_traces set status='sent',sent_at='now' where id=?1",
                    [TRACE_ID],
                ),
                "expired" => connection.execute(
                    "update concept_retrieval_traces set expires_at=?2 where id=?1",
                    params![TRACE_ID, PAST],
                ),
                "deleted" => connection.execute(
                    "delete from concept_retrieval_traces where id=?1",
                    [TRACE_ID],
                ),
                _ => unreachable!(),
            }
            .unwrap();
        }
        let mut connection = fixture.db.lock().unwrap();
        let tx = immediate(&mut connection);
        let error = consume_prepared_trace_in(&tx, &prepared, &ready()).unwrap_err();
        let TraceDataError::Data(error) = error else {
            panic!("expected changed snapshot")
        };
        assert_eq!(error.code, "plugin_data_snapshot_changed", "{change}");
        assert_eq!(error.phase, "recheck");
        tx.rollback().unwrap();
    }
}

#[test]
fn changed_snapshot_wins_over_a_stale_host_domain_rejection() {
    let fixture = DatabaseFixture::new();
    fixture.insert_trace("Original synthetic context", PAST);
    let prepared = fixture.capture(&PackageFixture::new(), "page");
    let mut connection = fixture.db.lock().unwrap();
    let tx = immediate(&mut connection);
    // Earlier caller work in this same T1 is visible at the consume position.
    tx.execute(
        "delete from concept_retrieval_traces where id=?1",
        [TRACE_ID],
    )
    .unwrap();
    let error = consume_prepared_trace_in(
        &tx,
        &prepared,
        &TraceDecision::Rejected {
            reason: TraceRejection::Expired,
        },
    )
    .unwrap_err();
    assert!(
        matches!(error, TraceDataError::Data(ref e) if e.code == "plugin_data_snapshot_changed")
    );
    tx.rollback().unwrap();
    assert_eq!(status(&connection), "previewed");
}

#[test]
fn failure_in_the_original_core_writer_rolls_back_mark_sent_on_reopen() {
    let fixture = DatabaseFixture::new();
    fixture.insert_trace("Atomic synthetic context", FUTURE);
    fixture
        .db
        .lock()
        .unwrap()
        .execute_batch(
            "create trigger simulate_core_failure before insert on note_pages
         begin select raise(abort,'private-core-insert-cause'); end;",
        )
        .unwrap();
    let prepared = fixture.capture(&PackageFixture::new(), "page");
    {
        let mut connection = fixture.db.lock().unwrap();
        let tx = immediate(&mut connection);
        consume_prepared_trace_in(&tx, &prepared, &ready()).unwrap();
        assert_eq!(status(&tx), "sent");
        assert!(core_note(&tx, "Must roll back").is_err());
        tx.rollback().unwrap();
    }
    let fixture = fixture.reopen();
    assert_eq!(fixture.status(), "previewed");
    let count: u64 = fixture
        .db
        .lock()
        .unwrap()
        .query_row("select count(*) from note_pages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn expire_sql_failure_precedes_expired_and_diagnostics_do_not_echo_private_values() {
    let fixture = DatabaseFixture::new();
    fixture.insert_trace("private-synthetic-context", PAST);
    fixture.db.lock().unwrap().execute_batch(
        "create trigger simulate_expire_failure before update of status on concept_retrieval_traces
         when new.status='expired' begin select raise(abort,'private-expire-cause'); end;",
    ).unwrap();
    let prepared = fixture.capture(&PackageFixture::new(), "page");
    let mut connection = fixture.db.lock().unwrap();
    let tx = immediate(&mut connection);
    let error = consume_prepared_trace_in(
        &tx,
        &prepared,
        &TraceDecision::Rejected {
            reason: TraceRejection::Expired,
        },
    )
    .unwrap_err();
    let TraceDataError::Data(error) = error else {
        panic!("SQL failure must precede Expired")
    };
    assert_eq!(error.code, "plugin_data_sql_failed");
    assert_eq!(
        error.sqlite_extended_code,
        Some(rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER)
    );
    assert_eq!(error.resource_id, "literature.retrieval.trace-expire");
    let safe = serde_json::to_string(&error).unwrap();
    for private in [
        TRACE_ID,
        QUERY,
        "private-synthetic-context",
        "private-expire-cause",
    ] {
        assert!(!safe.contains(private));
        assert!(!error.to_string().contains(private));
    }
    tx.rollback().unwrap();
}

#[test]
fn expired_keeps_original_update_then_error_even_when_update_affects_zero_rows() {
    for ignore_update in [false, true] {
        let fixture = DatabaseFixture::new();
        fixture.insert_trace("Expired synthetic context", PAST);
        if ignore_update {
            fixture.db.lock().unwrap().execute_batch(
                "create trigger ignore_expire before update of status on concept_retrieval_traces
                 when new.status='expired' begin select raise(ignore); end;",
            ).unwrap();
        }
        let prepared = fixture.capture(&PackageFixture::new(), "page");
        {
            let mut connection = fixture.db.lock().unwrap();
            let tx = immediate(&mut connection);
            let error = consume_prepared_trace_in(
                &tx,
                &prepared,
                &TraceDecision::Rejected {
                    reason: TraceRejection::Expired,
                },
            )
            .unwrap_err();
            assert!(matches!(
                error,
                TraceDataError::Rejected(TraceRejection::Expired)
            ));
            assert_eq!(
                status(&tx),
                if ignore_update {
                    "previewed"
                } else {
                    "expired"
                }
            );
            tx.rollback().unwrap();
        }
        assert_eq!(fixture.reopen().status(), "previewed");
    }
}

#[test]
fn core_source_factory_runs_before_mark_sent() {
    let fixture = DatabaseFixture::new();
    fixture.insert_trace("Synthetic context", FUTURE);
    fixture
        .db
        .lock()
        .unwrap()
        .execute_batch(
            "create trigger mark_would_fail before update of status on concept_retrieval_traces
         when new.status='sent' begin select raise(abort,'mark-must-not-run'); end;",
        )
        .unwrap();
    let prepared = fixture.capture(&PackageFixture::new(), " ");
    let mut connection = fixture.db.lock().unwrap();
    let tx = immediate(&mut connection);
    assert!(matches!(
        consume_prepared_trace_in(&tx, &prepared, &ready()),
        Err(TraceDataError::InvalidSource)
    ));
    assert_eq!(status(&tx), "previewed");
    tx.rollback().unwrap();
}

#[test]
fn non_expired_domain_rejections_are_preserved_without_writes() {
    let fixture = DatabaseFixture::new();
    fixture.insert_trace("Synthetic context", FUTURE);
    let prepared = fixture.capture(&PackageFixture::new(), "page");
    for reason in [
        TraceRejection::NotFound,
        TraceRejection::QueryMismatch,
        TraceRejection::AlreadyConsumed,
        TraceRejection::ContextHashMismatch,
    ] {
        let mut connection = fixture.db.lock().unwrap();
        let tx = immediate(&mut connection);
        assert!(matches!(
            consume_prepared_trace_in(&tx, &prepared, &TraceDecision::Rejected { reason }),
            Err(TraceDataError::Rejected(actual)) if actual == reason
        ));
        assert_eq!(status(&tx), "previewed");
        tx.rollback().unwrap();
    }
}

#[test]
fn ready_postcondition_cannot_be_changed_and_mark_zero_keeps_already_consumed() {
    let fixture = DatabaseFixture::new();
    fixture.insert_trace("Synthetic context", FUTURE);
    let prepared = fixture.capture(&PackageFixture::new(), "page");
    for mark_sent in [
        MarkSentCondition {
            expected_rows: 0,
            ..MarkSentCondition::ORIGINAL
        },
        MarkSentCondition {
            on_mismatch: TraceRejection::Expired,
            ..MarkSentCondition::ORIGINAL
        },
    ] {
        let mut connection = fixture.db.lock().unwrap();
        let tx = immediate(&mut connection);
        assert!(matches!(
            consume_prepared_trace_in(&tx, &prepared, &TraceDecision::Ready { mark_sent }),
            Err(TraceDataError::InvalidDecision)
        ));
        assert_eq!(status(&tx), "previewed");
        tx.rollback().unwrap();
    }
    fixture
        .db
        .lock()
        .unwrap()
        .execute_batch(
            "create trigger ignore_mark before update of status on concept_retrieval_traces
         when new.status='sent' begin select raise(ignore); end;",
        )
        .unwrap();
    let mut connection = fixture.db.lock().unwrap();
    let tx = immediate(&mut connection);
    assert!(matches!(
        consume_prepared_trace_in(&tx, &prepared, &ready()),
        Err(TraceDataError::Rejected(TraceRejection::AlreadyConsumed))
    ));
    tx.rollback().unwrap();
}

#[test]
fn original_large_nul_body_is_owned_complete_and_not_rejected_by_a_new_body_limit() {
    let fixture = DatabaseFixture::new();
    let body = format!("prefix\0{}", "x".repeat(64 * 1024 * 1024));
    fixture.insert_trace(&body, FUTURE);
    let prepared = fixture.capture(&PackageFixture::new(), "page");
    let snapshot = prepared.note_input().unwrap();
    assert_eq!(snapshot.row.as_ref().unwrap().context_text, body);
    assert!(snapshot.row.as_ref().unwrap().context_text.len() > 64 * 1024 * 1024);
    drop(snapshot);
    let mut connection = fixture.db.lock().unwrap();
    let tx = immediate(&mut connection);
    let source = consume_prepared_trace_in(&tx, &prepared, &ready()).unwrap();
    assert_eq!(source.text, body);
    assert_eq!(status(&tx), "sent");
    tx.rollback().unwrap();
}
