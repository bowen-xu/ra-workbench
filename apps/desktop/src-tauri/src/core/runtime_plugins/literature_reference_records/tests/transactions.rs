use super::*;
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization, TransactionOperation},
};

fn clear_authorizer(db: &Connection) {
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

#[test]
fn new_and_update_history_failures_roll_back_main_and_reopen_original_core_prefix() {
    for update in [false, true] {
        let fixture = Fixture::new();
        let prefix = fixture.prefix();
        let before = update.then(|| fixture.save(draft()).unwrap());
        fixture
            .db
            .lock()
            .unwrap()
            .execute_batch(
                "create temp trigger synthetic_reference_history_failure
             before insert on reference_record_revisions
             begin select raise(abort,'private-synthetic-history-trigger'); end;",
            )
            .unwrap();
        let failure = fixture
            .save(ReferenceRecordDraft {
                id: before.as_ref().map(|record| record.id.clone()),
                expected_revision: before.as_ref().map(|record| record.revision),
                raw_text: "private-synthetic-bound-body".into(),
                ..draft()
            })
            .unwrap_err();
        assert_eq!(failure.progress.phase, "history");
        assert_eq!(failure.progress.outcome, ReferenceOutcome::RolledBack);
        assert!(failure.rollback_error.is_none());
        uuid::Uuid::parse_str(failure.progress.record_id.as_deref().unwrap()).unwrap();
        let ReferenceProblem::Data(cause) = &failure.problem else {
            panic!("expected original revision INSERT failure");
        };
        assert_eq!(cause.resource_id, "literature.reference-records.revision");
        assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
        assert!(cause.sqlite_extended_code.is_some());
        for diagnostic in [
            serde_json::to_string(&failure).unwrap(),
            format!("{failure:?}"),
        ] {
            assert!(!diagnostic.contains("private-synthetic-history-trigger"));
            assert!(!diagnostic.contains("private-synthetic-bound-body"));
            assert!(!diagnostic.contains("insert into reference_record_revisions"));
        }
        assert!(fixture.db.lock().unwrap().is_autocommit());
        let fixture = fixture.reopen();
        assert_eq!(fixture.prefix(), prefix);
        assert_eq!(
            fixture.records("citing").unwrap(),
            before.into_iter().collect::<Vec<_>>()
        );
        assert_eq!(
            fixture.count("reference_record_revisions"),
            i64::from(update)
        );
    }
}

#[test]
fn ignored_update_is_original_revision_conflict_and_retains_main_and_history() {
    let fixture = Fixture::new();
    let created = fixture.save(draft()).unwrap();
    let prefix = fixture.prefix();
    fixture.db.lock().unwrap().execute_batch(
        "create temp trigger synthetic_reference_update_ignore before update on reference_records
         begin select raise(ignore); end;",
    ).unwrap();
    let failure = fixture
        .save(ReferenceRecordDraft {
            id: Some(created.id.clone()),
            expected_revision: Some(1),
            raw_text: "Uncommitted replacement".into(),
            ..draft()
        })
        .unwrap_err();
    assert_eq!(failure.progress.phase, "statement");
    assert_eq!(failure.progress.outcome, ReferenceOutcome::RolledBack);
    assert_eq!(failure.progress.current_revision, Some(1));
    assert!(matches!(
        failure.problem,
        ReferenceProblem::RevisionConflict
    ));
    let fixture = fixture.reopen();
    assert_eq!(fixture.records("citing").unwrap(), vec![created]);
    assert_eq!(fixture.count("reference_record_revisions"), 1);
    assert_eq!(fixture.prefix(), prefix);
}

#[test]
fn committed_fields_decode_failure_retains_actual_identity_without_replaying_save() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix();
    fixture.db.lock().unwrap().execute_batch(
        "create temp trigger synthetic_reference_readback_failure
         after insert on reference_record_revisions
         begin update reference_records set fields_json='{}' where id=new.reference_record_id; end;",
    ).unwrap();
    let failure = fixture.save(draft()).unwrap_err();
    assert_eq!(failure.progress.phase, "fields-codec");
    assert_eq!(failure.progress.outcome, ReferenceOutcome::Committed);
    assert!(failure.rollback_error.is_none());
    let record_id = failure.progress.record_id.clone().unwrap();
    uuid::Uuid::parse_str(&record_id).unwrap();
    assert!(matches!(
        failure.problem,
        ReferenceProblem::Domain(ReferenceRecordRejectionV1::StoredFieldsEncoding)
    ));
    let diagnostic = failure.native();
    assert_eq!(diagnostic.code, "invalid_input");
    assert!(diagnostic.message.contains("已提交") && diagnostic.message.contains("禁止自动重放"));
    assert!(fixture.db.lock().unwrap().is_autocommit());
    assert_eq!(fixture.count("reference_records"), 1);
    assert_eq!(fixture.count("reference_record_revisions"), 1);
    let fixture = fixture.reopen();
    let db = fixture.db.lock().unwrap();
    let saved: (String, String, i64) = db
        .query_row(
            "select id,fields_json,current_revision from reference_records",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(saved, (record_id, "{}".into(), 1));
    drop(db);
    assert_eq!(fixture.count("reference_record_revisions"), 1);
    assert_eq!(fixture.prefix(), prefix);
}

#[test]
fn rollback_failure_retains_primary_cause_fences_owner_and_reopens_original_prefix() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix();
    let mut db = fixture.db.lock().unwrap();
    db.execute_batch(
        "create temp trigger synthetic_reference_rollback_failure
         before insert on reference_record_revisions
         begin select raise(abort,'synthetic-original-history-error'); end;",
    )
    .unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Rollback,
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let failure = save(&mut db, &fixture.resources, &plan(draft())).unwrap_err();
    assert_eq!(failure.progress.phase, "history");
    assert_eq!(failure.progress.outcome, ReferenceOutcome::Unknown);
    let ReferenceProblem::Data(cause) = &failure.problem else {
        panic!("rollback must not replace the history cause");
    };
    assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
    let rollback = failure.rollback_error.unwrap();
    assert_eq!(rollback.phase, "rollback");
    assert_eq!(
        rollback.sqlite_code.as_deref(),
        Some("AuthorizationForStatementDenied")
    );
    assert!(!db.is_autocommit());
    clear_authorizer(&db);
    drop(db);
    assert!(fixture.db.lock().is_err());
    let fixture = fixture.reopen();
    assert_eq!(fixture.count("reference_records"), 0);
    assert_eq!(fixture.count("reference_record_revisions"), 0);
    assert_eq!(fixture.prefix(), prefix);
}

#[test]
fn failed_commit_without_rollback_receipt_stays_unknown_even_when_autocommit_is_true() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix();
    let mut db = fixture.db.lock().unwrap();
    db.commit_hook(Some(|| true));
    let failure = save(&mut db, &fixture.resources, &plan(draft())).unwrap_err();
    assert_eq!(failure.progress.phase, "commit");
    assert_eq!(failure.progress.outcome, ReferenceOutcome::Unknown);
    assert!(failure.rollback_error.is_none());
    let ReferenceProblem::Database(cause) = failure.problem else {
        panic!("expected original COMMIT failure");
    };
    assert_eq!(cause.phase, "commit");
    assert_eq!(cause.sqlite_code.as_deref(), Some("ConstraintViolation"));
    assert!(db.is_autocommit());
    db.commit_hook(None::<fn() -> bool>);
    drop(db);
    assert!(fixture.db.lock().is_err());
    let fixture = fixture.reopen();
    assert_eq!(fixture.count("reference_records"), 0);
    assert_eq!(fixture.count("reference_record_revisions"), 0);
    assert_eq!(fixture.prefix(), prefix);
}

#[test]
fn original_new_update_identity_and_revision_rejections_leave_no_partial_writes() {
    enum Expected {
        Domain(ReferenceRecordRejectionV1),
        Missing,
        Conflict,
    }
    let fixture = Fixture::new();
    let created = fixture.save(draft()).unwrap();
    let prefix = fixture.prefix();
    for (value, phase, expected) in [
        (
            ReferenceRecordDraft {
                expected_revision: Some(1),
                ..draft()
            },
            "revision",
            Expected::Domain(ReferenceRecordRejectionV1::NewExpectedRevision),
        ),
        (
            ReferenceRecordDraft {
                id: Some("missing".into()),
                expected_revision: Some(1),
                ..draft()
            },
            "header",
            Expected::Missing,
        ),
        (
            ReferenceRecordDraft {
                id: Some(created.id.clone()),
                source_anchor_id: "usage-anchor".into(),
                expected_revision: Some(1),
                ..draft()
            },
            "identity",
            Expected::Domain(ReferenceRecordRejectionV1::ImmutableOwner),
        ),
        (
            ReferenceRecordDraft {
                id: Some(created.id.clone()),
                citing_paper_id: "other".into(),
                source_anchor_id: "other-anchor".into(),
                expected_revision: Some(1),
                ..draft()
            },
            "identity",
            Expected::Domain(ReferenceRecordRejectionV1::ImmutableOwner),
        ),
        (
            ReferenceRecordDraft {
                id: Some(created.id.clone()),
                expected_revision: Some(0),
                ..draft()
            },
            "revision",
            Expected::Conflict,
        ),
        (
            ReferenceRecordDraft {
                id: Some(created.id.clone()),
                expected_revision: None,
                ..draft()
            },
            "revision",
            Expected::Conflict,
        ),
        (
            ReferenceRecordDraft {
                id: Some(created.id.clone()),
                expected_revision: Some(-1),
                ..draft()
            },
            "revision",
            Expected::Conflict,
        ),
    ] {
        let expected_id = value.id.clone();
        let failure = fixture.save(value).unwrap_err();
        assert_eq!(failure.progress.phase, phase);
        assert_eq!(failure.progress.outcome, ReferenceOutcome::RolledBack);
        assert_eq!(failure.progress.record_id, expected_id);
        match (expected, failure.problem) {
            (Expected::Domain(expected), ReferenceProblem::Domain(actual)) => {
                assert_eq!(actual, expected)
            }
            (Expected::Missing, ReferenceProblem::Missing) => {}
            (Expected::Conflict, ReferenceProblem::RevisionConflict) => {
                assert_eq!(failure.progress.current_revision, Some(1))
            }
            _ => panic!("the original business rejection changed"),
        }
        assert!(fixture.db.lock().unwrap().is_autocommit());
    }
    assert_eq!(fixture.records("citing").unwrap(), vec![created.clone()]);
    assert_eq!(fixture.count("reference_record_revisions"), 1);
    assert_eq!(fixture.prefix(), prefix);

    fixture
        .db
        .lock()
        .unwrap()
        .execute(
            "update reference_records set current_revision=1.5 where id=?1",
            [&created.id],
        )
        .unwrap();
    let failure = fixture
        .save(ReferenceRecordDraft {
            id: Some(created.id),
            source_anchor_id: "usage-anchor".into(),
            expected_revision: Some(99),
            ..draft()
        })
        .unwrap_err();
    assert_eq!(failure.progress.phase, "header");
    assert_eq!(failure.progress.outcome, ReferenceOutcome::RolledBack);
    assert_eq!(failure.progress.current_revision, None);
    let ReferenceProblem::Data(cause) = failure.problem else {
        panic!("actual header codec must precede identity and CAS");
    };
    assert_eq!(cause.resource_id, "literature.reference-records.header");
    assert_eq!(cause.safe_cause, "invalid_column_type");
    assert!(fixture.db.lock().unwrap().is_autocommit());
}
