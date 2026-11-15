use super::*;
use research_workbench_native_sdk::reading_units::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
mod fixture;
mod transactions;
use fixture::{Fixture, draft, plan};

#[test]
fn create_update_keep_original_manual_history_evidence_identity_and_cold_body() {
    let f = Fixture::new();
    let note = f.note();
    let mut d = draft();
    d.anchor_id = Some("anchor-a".into());
    let created = f.save(plan(d)).unwrap();
    assert_eq!(created.revision, 1);
    assert_eq!(created.evidence_count, 1);
    assert_eq!(created.paper_title.as_deref(), Some("Paper a"));
    let link: (String, String) =
        f.db.lock()
            .unwrap()
            .query_row(
                "select id,created_at from reading_unit_evidence_links",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
    f.db.lock().unwrap().execute("update structured_reading_units set provenance_kind='ai_accepted_unchanged' where id=?1",[&created.id]).unwrap();
    let mut d = draft();
    d.id = Some(created.id.clone());
    d.expected_revision = Some(1);
    d.body = "  Edited body  ".into();
    d.anchor_id = Some("anchor-a".into());
    d.evidence_rationale = Some("  rationale  ".into());
    d.change_reason = Some(" ".into());
    let updated = f.save(plan(d)).unwrap();
    assert_eq!(updated.body, "Edited body");
    assert_eq!(updated.provenance_kind, "ai_accepted_unchanged");
    assert_eq!(updated.evidence_count, 1);
    let db = f.db.lock().unwrap();
    let actual: (String, String, String) = db
        .query_row(
            "select id,created_at,rationale from reading_unit_evidence_links",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((actual.0, actual.1), link);
    assert_eq!(actual.2, "rationale");
    let (snapshot,reason):(String,String)=db.query_row("select snapshot_json,change_reason from research_object_revisions where object_kind='reading_unit' and object_id=?1 and revision=2",[&created.id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    assert_eq!(snapshot["provenanceKind"], "human_original");
    assert_eq!(snapshot.as_object().unwrap().len(), 9);
    assert_eq!(reason, " ");
    drop(db);
    let mut d = draft();
    d.id = Some(created.id);
    d.expected_revision = Some(2);
    f.save(plan(d)).unwrap(); // None anchor retains evidence.
    let before = f.state();
    assert_eq!(f.note(), note);
    let f = f.reopen();
    assert_eq!(f.state(), before);
    assert_eq!(f.note(), note);
}
#[test]
fn optional_evidence_fk_rolls_back_main_and_history_but_project_fk_fails_first() {
    let f = Fixture::new();
    let before = f.state();
    let mut d = draft();
    d.anchor_id = Some("missing".into());
    let e = f.save(plan(d)).unwrap_err();
    assert_eq!(e.phase, "evidence-upsert");
    assert_eq!(e.outcome, ReadingUnitOutcome::RolledBack);
    assert!(e.reading_unit_id.is_some());
    assert_eq!(f.state(), before);
    let mut d = draft();
    d.project_id = Some("missing-project".into());
    d.anchor_id = Some("missing".into());
    let e = f.save(plan(d)).unwrap_err();
    assert_eq!(e.phase, "insert");
    assert_eq!(e.outcome, ReadingUnitOutcome::RolledBack);
    assert_eq!(f.state(), before);
}
#[test]
fn begin_precedes_new_expected_refusal_and_missing_precedes_cas() {
    let f = Fixture::new();
    let mut d = draft();
    d.expected_revision = Some(1);
    f.db.lock()
        .unwrap()
        .execute_batch("BEGIN DEFERRED")
        .unwrap();
    let e = f.save(plan(d.clone())).unwrap_err();
    assert_eq!(e.phase, "begin");
    assert_eq!(e.outcome, ReadingUnitOutcome::NotStarted);
    assert!(e.reading_unit_id.is_none());
    f.db.lock().unwrap().execute_batch("ROLLBACK").unwrap();
    let e = f.save(plan(d)).unwrap_err();
    assert_eq!(e.outcome, ReadingUnitOutcome::RolledBack);
    assert!(matches!(
        e.problem.as_ref(),
        ReadingUnitProblem::Domain(ReadingUnitRejectionV1::NewExpectedRevision)
    ));
    let mut d = draft();
    d.id = Some("missing-unit".into());
    d.expected_revision = None;
    let e = f.save(plan(d)).unwrap_err();
    assert_eq!(e.phase, "current-revision");
    assert!(matches!(e.problem.as_ref(), ReadingUnitProblem::Missing));
    assert_eq!(e.reading_unit_id.as_deref(), Some("missing-unit"));
    assert!(e.current_revision.is_none());
}
#[test]
fn stale_cas_retains_actual_revision_and_does_not_write() {
    let f = Fixture::new();
    let original = f.save(plan(draft())).unwrap();
    let before = f.state();
    let mut d = draft();
    d.id = Some(original.id.clone());
    d.expected_revision = Some(0);
    let e = f.save(plan(d)).unwrap_err();
    assert_eq!(e.current_revision, Some(1));
    assert_eq!(e.reading_unit_id.as_deref(), Some(original.id.as_str()));
    assert_eq!(e.outcome, ReadingUnitOutcome::RolledBack);
    assert_eq!(f.state(), before);
}
#[test]
fn full_optional_template_read_occurs_only_after_commit() {
    let f = Fixture::new();
    f.db.lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "reading_unit_template_fields",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let e = f.save(plan(draft())).unwrap_err();
    assert_eq!(e.phase, "postcommit-read");
    assert_eq!(e.outcome, ReadingUnitOutcome::Committed);
    let id = e.reading_unit_id.unwrap();
    f.db.lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let db = f.db.lock().unwrap();
    assert_eq!(
        db.query_row(
            "select revision from structured_reading_units where id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "select count(*) from research_object_revisions where object_kind='reading_unit'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn list_keeps_and_filters_order_and_fails_on_bad_rows_instead_of_skipping() {
    let f = Fixture::new();
    let a = f.save(plan(draft())).unwrap();
    let b = f.save(plan(draft())).unwrap();
    f.db.lock()
        .unwrap()
        .execute("update structured_reading_units set updated_at='same'", [])
        .unwrap();
    let read = |project_id, paper_id| {
        f.apply(ReadingUnitPreparationV1::Ready {
            plan: Box::new(ReadingUnitPlanV1::List {
                project_id,
                paper_id,
            }),
        })
    };
    let ReadingUnitResult::Units(units) = read(None, Some("paper-a".into())).unwrap() else {
        panic!("list")
    };
    let mut expected = vec![a.id.clone(), b.id];
    expected.sort_by(|a, b| b.cmp(a));
    assert_eq!(
        units.iter().map(|u| u.id.clone()).collect::<Vec<_>>(),
        expected
    );
    let ReadingUnitResult::Units(units) = read(Some("".into()), Some("paper-a".into())).unwrap()
    else {
        panic!("list")
    };
    assert!(units.is_empty());
    f.db.lock()
        .unwrap()
        .execute(
            "update structured_reading_units set body=cast(x'FF' as text) where id=?1",
            [&a.id],
        )
        .unwrap();
    let e = read(None, None).err().expect("malformed row");
    assert_eq!(e.phase, "read");
    assert_eq!(e.reading_unit_id.as_deref(), Some(a.id.as_str()));
}
#[test]
fn finite_preparation_binding_rejects_changed_filters_or_original_draft() {
    let input = ReadingUnitRequestV1::List {
        project_id: None,
        paper_id: Some("p".into()),
    };
    let result = ReadingUnitPreparationV1::Ready {
        plan: Box::new(ReadingUnitPlanV1::List {
            project_id: None,
            paper_id: None,
        }),
    };
    assert!(check_preparation(&input, &result).is_err());
    let mut original = draft();
    let result = plan(original.clone());
    original.title = "changed".into();
    assert!(
        check_preparation(
            &ReadingUnitRequestV1::Save {
                draft: Box::new(original)
            },
            &result
        )
        .is_err()
    );
}
