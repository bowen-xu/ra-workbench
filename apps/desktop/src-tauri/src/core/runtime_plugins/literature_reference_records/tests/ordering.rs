use super::*;
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization, TransactionOperation},
    types::Type,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn clear_authorizer(db: &Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

#[test]
fn prebegin_anchor_rejections_precede_later_matched_sql_and_never_begin() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix();
    let matched_reads = Arc::new(AtomicUsize::new(0));
    let begins = Arc::new(AtomicUsize::new(0));
    let mut db = fixture.db.lock().unwrap();
    let observed_reads = Arc::clone(&matched_reads);
    let observed_begins = Arc::clone(&begins);
    db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "bibliographic_items",
            ..
        } => {
            observed_reads.fetch_add(1, Ordering::SeqCst);
            Authorization::Deny
        }
        AuthAction::Transaction {
            operation: TransactionOperation::Begin,
        } => {
            observed_begins.fetch_add(1, Ordering::SeqCst);
            Authorization::Allow
        }
        _ => Authorization::Allow,
    }));
    let base = ReferenceRecordDraft {
        identity_state: "matched_existing".into(),
        matched_item_id: Some("cited".into()),
        semantic_state: "reviewed".into(),
        usage_kind: Some("supports".into()),
        usage_summary: "Synthetic assessment".into(),
        usage_anchor_id: Some("usage-anchor".into()),
        ..draft()
    };
    for (value, phase) in [
        (
            ReferenceRecordDraft {
                source_anchor_id: "other-anchor".into(),
                ..base.clone()
            },
            "source-anchor",
        ),
        (
            ReferenceRecordDraft {
                usage_anchor_id: Some("other-anchor".into()),
                ..base.clone()
            },
            "usage-anchor",
        ),
    ] {
        let failure = save(&mut db, &fixture.resources, &plan(value)).unwrap_err();
        assert_eq!(failure.progress.phase, phase);
        assert_eq!(failure.progress.outcome, ReferenceOutcome::NotStarted);
        assert!(matches!(
            failure.problem,
            ReferenceProblem::Domain(ReferenceRecordRejectionV1::AnchorOwnership)
        ));
        assert_eq!(matched_reads.load(Ordering::SeqCst), 0);
    }
    let failure = save(&mut db, &fixture.resources, &plan(base)).unwrap_err();
    assert_eq!(failure.progress.phase, "matched-item");
    assert_eq!(failure.progress.outcome, ReferenceOutcome::NotStarted);
    let ReferenceProblem::Data(cause) = failure.problem else {
        panic!("expected actual matched query denial");
    };
    assert_eq!(
        cause.resource_id,
        "literature.reference-records.matched-exists"
    );
    assert_eq!(
        cause.sqlite_code.as_deref(),
        Some("AuthorizationForStatementDenied")
    );
    assert!(matched_reads.load(Ordering::SeqCst) > 0);
    assert_eq!(begins.load(Ordering::SeqCst), 0);
    assert!(db.is_autocommit());
    clear_authorizer(&db);
    drop(db);
    assert_eq!(fixture.count("reference_records"), 0);
    assert_eq!(fixture.prefix(), prefix);
}

#[test]
fn begin_failure_precedes_new_expected_revision_and_all_update_header_policies() {
    let fixture = Fixture::new();
    let created = fixture.save(draft()).unwrap();
    let prefix = fixture.prefix();
    let cases = [
        ReferenceRecordDraft {
            expected_revision: Some(9),
            ..draft()
        },
        ReferenceRecordDraft {
            id: Some("missing".into()),
            expected_revision: Some(1),
            ..draft()
        },
        ReferenceRecordDraft {
            id: Some(created.id.clone()),
            source_anchor_id: "usage-anchor".into(),
            expected_revision: Some(1),
            ..draft()
        },
        ReferenceRecordDraft {
            id: Some(created.id.clone()),
            expected_revision: Some(9),
            ..draft()
        },
    ];
    let mut db = fixture.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Begin,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    for value in cases {
        let failure = save(&mut db, &fixture.resources, &plan(value)).unwrap_err();
        assert_eq!(failure.progress.phase, "begin");
        assert_eq!(failure.progress.outcome, ReferenceOutcome::NotStarted);
        let ReferenceProblem::Database(cause) = failure.problem else {
            panic!("BEGIN must win over the later domain branch");
        };
        assert_eq!(
            cause.sqlite_code.as_deref(),
            Some("AuthorizationForStatementDenied")
        );
        assert!(db.is_autocommit());
    }
    clear_authorizer(&db);
    drop(db);
    assert_eq!(fixture.count("reference_records"), 1);
    assert_eq!(fixture.count("reference_record_revisions"), 1);
    assert_eq!(fixture.prefix(), prefix);
}

#[test]
fn list_collects_all_id_codecs_before_reading_an_earlier_bad_fields_record() {
    let fixture = Fixture::new();
    let created = fixture.save(draft()).unwrap();
    {
        let db = fixture.db.lock().unwrap();
        db.execute(
            "update reference_records set fields_json='{}',updated_at='z' where id=?1",
            [&created.id],
        )
        .unwrap();
        // SQLite's non-STRICT TEXT primary key permits this BLOB. It has valid
        // foreign keys and needs no constraint, schema or migration changes.
        db.execute_batch(
            "insert into reference_records
             (id,citing_paper_id,source_anchor_id,raw_text,fields_json,parse_state,
              identity_state,semantic_state,usage_summary,provenance_kind,
              current_revision,created_at,updated_at)
             values(x'01','citing','reference-anchor','Later synthetic record','[]',
              'unparsed','unverified','unreviewed','','human_original',1,'a','a');",
        )
        .unwrap();
    }
    let failure = fixture.records("citing").unwrap_err();
    assert_eq!(failure.progress.phase, "list-ids");
    assert_eq!(failure.progress.outcome, ReferenceOutcome::NotStarted);
    let ReferenceProblem::Data(cause) = failure.problem else {
        panic!("the later ID codec must precede every main read");
    };
    assert_eq!(cause.resource_id, "literature.reference-records.ids");
    assert_eq!(cause.safe_cause, "invalid_column_type");
}

#[test]
fn list_decodes_each_fields_json_before_later_main_row_conversion() {
    let fixture = Fixture::new();
    let first = fixture.save(draft()).unwrap();
    let second = fixture.save(draft()).unwrap();
    {
        let db = fixture.db.lock().unwrap();
        db.execute(
            "update reference_records set fields_json='{}',updated_at='z' where id=?1",
            [&first.id],
        )
        .unwrap();
        db.execute(
            "update reference_records set current_revision=1.5,updated_at='a' where id=?1",
            [&second.id],
        )
        .unwrap();
    }
    let failure = fixture.records("citing").unwrap_err();
    assert_eq!(failure.progress.phase, "fields-codec");
    assert_eq!(
        failure.progress.record_id.as_deref(),
        Some(first.id.as_str())
    );
    assert!(matches!(
        failure.problem,
        ReferenceProblem::Domain(ReferenceRecordRejectionV1::StoredFieldsEncoding)
    ));
}

#[test]
fn record_decodes_remaining_columns_before_fields_json_column_four() {
    let fixture = Fixture::new();
    let created = fixture.save(draft()).unwrap();
    let db = fixture.db.lock().unwrap();
    db.execute(
        "update reference_records set current_revision=1.5 where id=?1",
        [&created.id],
    )
    .unwrap();
    // Produce a NULL read for column 4 without corrupting its NOT NULL storage.
    // Both it and the original REAL revision fail row.get, and index 13 wins.
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "reference_records",
            column_name: "fields_json",
            ..
        } => Authorization::Ignore,
        _ => Authorization::Allow,
    }));
    let failure = list(&db, &fixture.resources, "citing").unwrap_err();
    clear_authorizer(&db);
    assert_eq!(failure.progress.phase, "list-record");
    let ReferenceProblem::Data(cause) = failure.problem else {
        panic!("expected original primitive row error");
    };
    assert_eq!(cause.safe_cause, "invalid_column_type");
    assert_eq!(
        cause.original_sql_failure.as_ref().unwrap().message(),
        rusqlite::Error::InvalidColumnType(13, "current_revision".into(), Type::Real).to_string(),
    );
}
