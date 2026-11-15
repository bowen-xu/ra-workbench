use super::*;
use research_workbench_native_sdk::literature_lifecycle::LifecycleRequestV1 as Request;
use rusqlite::{
    Connection,
    functions::FunctionFlags,
    hooks::{AuthAction, AuthContext, Authorization, TransactionOperation},
    params,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

mod fixture;
mod transactions;
use fixture::Fixture;

fn record_order(db: &Connection) -> Arc<Mutex<Vec<String>>> {
    let order = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&order);
    db.create_scalar_function(
        "synthetic_lifecycle_record",
        1,
        FunctionFlags::SQLITE_UTF8,
        move |context| {
            received.lock().unwrap().push(context.get::<String>(0)?);
            Ok(0_i64)
        },
    )
    .unwrap();
    order
}

#[test]
fn four_single_operations_keep_original_eligibility_relations_and_core_note_after_reopen() {
    let requests = [
        Request::Archive {
            item_id: "survivor".into(),
        },
        Request::Restore {
            item_id: "survivor".into(),
        },
        Request::Trash {
            item_id: "survivor".into(),
        },
        Request::RestoreTrash {
            item_id: "survivor".into(),
        },
    ];
    let accepted = [
        [true, false, false, false],
        [false, true, false, false],
        [true, false, false, false],
        [false, false, true, true],
    ];
    for (index, request) in requests.into_iter().enumerate() {
        let fixture = Fixture::new();
        let note = fixture.data.note();
        let relations = fixture.relations();
        let projects = fixture.state()["projects"].clone();
        for (state, (archived, trashed)) in [
            (None, None),
            (Some("archive"), None),
            (None, Some("trash")),
            (Some("archive"), Some("trash")),
        ]
        .into_iter()
        .enumerate()
        {
            fixture.data.db.lock().unwrap().execute(
                "update bibliographic_items set archived_at=?1,trashed_at=?2 where id='survivor'",
                params![archived, trashed],
            ).unwrap();
            let before = fixture.state();
            let result = fixture.apply(&request, &[]);
            if accepted[index][state] {
                result.unwrap();
                let flags = fixture.flags("survivor");
                match request {
                    Request::Archive { .. } => {
                        assert!(flags.0.is_some());
                        assert!(flags.1.is_none());
                    }
                    Request::Trash { .. } => {
                        assert!(flags.0.is_none());
                        assert!(flags.1.is_some());
                    }
                    _ => assert_eq!(flags, (None, None)),
                }
            } else {
                let error = result.unwrap_err();
                assert!(matches!(error.problem, LifecycleProblem::Missing));
                assert_eq!(
                    error.outcome,
                    LifecycleOutcome::StatementReturned { affected_rows: 0 }
                );
                assert!(error.rollback_error.is_none());
                assert_eq!(fixture.state(), before);
            }
            assert_eq!(fixture.relations(), relations);
            assert_eq!(fixture.state()["projects"], projects);
            assert_eq!(fixture.data.note(), note);
        }
        let missing = match request {
            Request::Archive { .. } => Request::Archive {
                item_id: "missing".into(),
            },
            Request::Restore { .. } => Request::Restore {
                item_id: "missing".into(),
            },
            Request::Trash { .. } => Request::Trash {
                item_id: "missing".into(),
            },
            _ => Request::RestoreTrash {
                item_id: "missing".into(),
            },
        };
        let before = fixture.state();
        assert!(matches!(
            fixture.apply(&missing, &[]).unwrap_err().problem,
            LifecycleProblem::Missing
        ));
        assert_eq!(fixture.reopen().state(), before);
    }
}

#[test]
fn single_ignore_and_fail_keep_actual_autocommit_effects_without_replay() {
    for ignored in [true, false] {
        let fixture = Fixture::new();
        let note = fixture.data.note();
        let relations = fixture.relations();
        let begins = Arc::new(AtomicUsize::new(0));
        {
            let db = fixture.data.db.lock().unwrap();
            let raise = if ignored {
                "raise(ignore)"
            } else {
                "raise(fail,'private lifecycle trigger text')"
            };
            db.execute_batch(&format!(
                "create temp trigger synthetic_single_effect before update of archived_at on bibliographic_items
                 when old.id='survivor' begin
                 insert into synthetic_lifecycle_effects(value) values('one-dispatch');
                 select {raise};end;"
            )).unwrap();
            let count = Arc::clone(&begins);
            db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
                AuthAction::Transaction {
                    operation: TransactionOperation::Begin,
                } => {
                    count.fetch_add(1, Ordering::SeqCst);
                    Authorization::Deny
                }
                _ => Authorization::Allow,
            }));
        }
        let error = fixture
            .apply(
                &Request::Archive {
                    item_id: "survivor".into(),
                },
                &[],
            )
            .unwrap_err();
        if ignored {
            assert!(matches!(error.problem, LifecycleProblem::Missing));
            assert_eq!(
                error.outcome,
                LifecycleOutcome::StatementReturned { affected_rows: 0 }
            );
        } else {
            assert_eq!(error.outcome, LifecycleOutcome::StatementFailed);
            let LifecycleProblem::Data(cause) = &error.problem else {
                panic!("original FAIL cause must survive");
            };
            assert_eq!(cause.sqlite_extended_code, Some(1811));
            assert_eq!(cause.resource_id, "literature.lifecycle.archive");
            assert_eq!(error.clone().native(false).code, "storage_error");
            assert!(
                !serde_json::to_string(&error)
                    .unwrap()
                    .contains("private lifecycle trigger text")
            );
        }
        assert!(error.rollback_error.is_none());
        {
            let db = fixture.data.db.lock().unwrap();
            assert!(db.is_autocommit());
            db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        }
        assert_eq!(begins.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.flags("survivor"), (None, None));
        assert_eq!(fixture.effects(), ["one-dispatch"]);
        assert_eq!(fixture.relations(), relations);
        assert_eq!(fixture.data.note(), note);
        let before = fixture.state();
        assert_eq!(fixture.reopen().state(), before);
    }
}

#[test]
fn complete_request_and_ordered_unique_batch_binding_reject_before_any_begin() {
    let request = Request::TrashMany {
        item_ids: vec!["survivor".into(), "source".into(), "source".into()],
    };
    check_preparation(&request, &request, &["source".into(), "survivor".into()]).unwrap();
    for ids in [
        vec![],
        vec!["source".into()],
        vec!["survivor".into(), "source".into()],
        vec!["source".into(), "source".into(), "survivor".into()],
        vec!["absent".into(), "source".into(), "survivor".into()],
    ] {
        let error = check_preparation(&request, &request, &ids).unwrap_err();
        assert!(matches!(error.problem, LifecycleProblem::Binding));
        assert_eq!(error.outcome, LifecycleOutcome::NotDispatched);
    }
    let changed = Request::TrashMany {
        item_ids: vec!["source".into(), "survivor".into()],
    };
    assert!(check_preparation(&request, &changed, &["source".into(), "survivor".into()]).is_err());
    let fixture = Fixture::new();
    let before = fixture.state();
    let begins = Arc::new(AtomicUsize::new(0));
    let mut db = fixture.data.db.lock().unwrap();
    let count = Arc::clone(&begins);
    db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
        AuthAction::Transaction {
            operation: TransactionOperation::Begin,
        } => {
            count.fetch_add(1, Ordering::SeqCst);
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }));
    // Empty requests are rejected by Native; Core also refuses an empty Ready.
    let error = apply(
        &mut db,
        &fixture.resources,
        &Request::TrashMany { item_ids: vec![] },
        &[],
    )
    .unwrap_err();
    assert_eq!(error.outcome, LifecycleOutcome::NotDispatched);
    assert_eq!(begins.load(Ordering::SeqCst), 0);
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    drop(db);
    assert_eq!(fixture.state(), before);
}

#[test]
fn batch_uses_sorted_unique_ids_once_and_keeps_classification_and_note() {
    let fixture = Fixture::new();
    let note = fixture.data.note();
    let relations = fixture.relations();
    fixture.data.db.lock().unwrap().execute_batch(
        "create temp trigger synthetic_batch_order before update of trashed_at on bibliographic_items
         begin insert into synthetic_lifecycle_effects(value) values(new.id);end;"
    ).unwrap();
    fixture
        .apply(
            &Request::TrashMany {
                item_ids: vec!["survivor".into(), "source".into(), "survivor".into()],
            },
            &["source".into(), "survivor".into()],
        )
        .unwrap();
    assert_eq!(fixture.effects(), ["source", "survivor"]);
    assert!(fixture.flags("source").1.is_some());
    assert!(fixture.flags("survivor").1.is_some());
    assert_eq!(fixture.relations(), relations);
    assert_eq!(fixture.data.note(), note);
    let before = fixture.state();
    assert_eq!(fixture.reopen().state(), before);
}

#[test]
fn batch_first_missing_or_late_abort_and_fail_roll_back_all_prior_effects() {
    for mode in ["missing-first", "missing-last", "abort", "fail"] {
        let fixture = Fixture::new();
        let before = fixture.state();
        let order = {
            let db = fixture.data.db.lock().unwrap();
            let order = record_order(&db);
            let raise = match mode {
                "abort" => "raise(abort,'private batch text')",
                "fail" => "raise(fail,'private batch text')",
                _ => "0",
            };
            db.execute_batch(&format!(
                "create temp trigger synthetic_batch_failure before update of trashed_at on bibliographic_items begin
                 select synthetic_lifecycle_record(new.id);
                 insert into synthetic_lifecycle_effects(value) values(new.id);
                 select case when new.id='survivor' then {raise} else 0 end;end;"
            )).unwrap();
            order
        };
        let ids: Vec<String> = match mode {
            "missing-first" => vec!["a-missing", "source", "survivor"],
            "missing-last" => vec!["source", "survivor", "z-missing"],
            _ => vec!["source", "survivor"],
        }
        .into_iter()
        .map(String::from)
        .collect();
        let mut original = ids.clone();
        original.reverse();
        original.push("source".into());
        let error = fixture
            .apply(&Request::TrashMany { item_ids: original }, &ids)
            .unwrap_err();
        assert_eq!(error.outcome, LifecycleOutcome::RolledBack);
        assert!(error.rollback_error.is_none());
        if mode.starts_with("missing") {
            assert!(matches!(error.problem, LifecycleProblem::Missing));
        } else {
            let LifecycleProblem::Data(cause) = &error.problem else {
                panic!("actual second-row SQL failure must win");
            };
            assert_eq!(cause.sqlite_extended_code, Some(1811));
            assert_eq!(cause.resource_id, "literature.lifecycle.trash");
        }
        let expected: &[&str] = if mode == "missing-first" {
            &[]
        } else {
            &["source", "survivor"]
        };
        assert_eq!(order.lock().unwrap().as_slice(), expected);
        assert_eq!(fixture.state(), before);
        assert_eq!(fixture.reopen().state(), before);
    }
}

#[test]
fn set_projects_keeps_vec_order_and_empty_or_missing_item_empty_still_commits() {
    let fixture = Fixture::new();
    let note = fixture.data.note();
    let relations = fixture.relations();
    let order = {
        let db = fixture.data.db.lock().unwrap();
        let order = record_order(&db);
        db.execute_batch(
            "create temp trigger synthetic_project_order before insert on project_items begin
            select synthetic_lifecycle_record(new.project_id);end;",
        )
        .unwrap();
        order
    };
    fixture
        .apply(
            &Request::SetProjects {
                item_id: "survivor".into(),
                project_ids: vec!["p-b".into(), "p-a".into()],
            },
            &[],
        )
        .unwrap();
    assert_eq!(order.lock().unwrap().as_slice(), ["p-b", "p-a"]);
    assert_eq!(
        fixture::rows(
            &fixture.data.db.lock().unwrap(),
            "select project_id from project_items where item_id='survivor' order by project_id"
        ),
        ["p-a", "p-b"]
    );
    fixture
        .apply(
            &Request::SetProjects {
                item_id: "survivor".into(),
                project_ids: vec![],
            },
            &[],
        )
        .unwrap();
    fixture
        .apply(
            &Request::SetProjects {
                item_id: "missing".into(),
                project_ids: vec![],
            },
            &[],
        )
        .unwrap();
    assert_eq!(fixture.state()["projects"], serde_json::json!([]));
    assert_eq!(fixture.relations(), relations);
    assert_eq!(fixture.data.note(), note);
    let before = fixture.state();
    assert_eq!(fixture.reopen().state(), before);
}

#[test]
fn project_duplicates_and_fk_follow_original_vec_order_and_restore_deleted_memberships() {
    struct Case {
        item: &'static str,
        ids: &'static [&'static str],
        code: i32,
        visited: &'static [&'static str],
    }
    for case in [
        Case {
            item: "survivor",
            ids: &["p-a", "p-a", "missing"],
            code: 1555,
            visited: &["p-a", "p-a"],
        },
        Case {
            item: "survivor",
            ids: &["missing", "p-a", "p-a"],
            code: 787,
            visited: &["missing"],
        },
        Case {
            item: "missing",
            ids: &["p-a"],
            code: 787,
            visited: &["p-a"],
        },
    ] {
        let fixture = Fixture::new();
        let before = fixture.state();
        let order = {
            let db = fixture.data.db.lock().unwrap();
            let order = record_order(&db);
            db.execute_batch(
                "create temp trigger synthetic_project_order before insert on project_items begin
                select synthetic_lifecycle_record(new.project_id);end;",
            )
            .unwrap();
            order
        };
        let error = fixture
            .apply(
                &Request::SetProjects {
                    item_id: case.item.into(),
                    project_ids: case.ids.iter().map(|id| (*id).into()).collect(),
                },
                &[],
            )
            .unwrap_err();
        assert_eq!(error.outcome, LifecycleOutcome::RolledBack);
        assert!(error.rollback_error.is_none());
        let LifecycleProblem::Data(cause) = &error.problem else {
            panic!("original ordered INSERT must supply the cause");
        };
        assert_eq!(cause.resource_id, "literature.lifecycle.project-insert");
        assert_eq!(cause.sqlite_extended_code, Some(case.code));
        assert_eq!(order.lock().unwrap().as_slice(), case.visited);
        assert_eq!(fixture.state(), before);
        assert_eq!(fixture.reopen().state(), before);
    }
}

#[test]
fn project_delete_failure_precedes_any_insert_and_rolls_back_trigger_writes() {
    let fixture = Fixture::new();
    let before = fixture.state();
    let order = {
        let db = fixture.data.db.lock().unwrap();
        let order = record_order(&db);
        db.execute_batch("create temp trigger synthetic_project_delete before delete on project_items begin
            select synthetic_lifecycle_record('delete');
            insert into synthetic_lifecycle_effects(value) values('delete-effect');
            select raise(fail,'private delete failure');end;
            create temp trigger synthetic_project_insert before insert on project_items begin
            select synthetic_lifecycle_record('insert');select raise(abort,'later insert failure');end;").unwrap();
        order
    };
    let error = fixture
        .apply(
            &Request::SetProjects {
                item_id: "survivor".into(),
                project_ids: vec!["p-a".into()],
            },
            &[],
        )
        .unwrap_err();
    assert_eq!(error.outcome, LifecycleOutcome::RolledBack);
    let LifecycleProblem::Data(cause) = error.problem else {
        panic!("DELETE must fail before INSERT");
    };
    assert_eq!(cause.resource_id, "literature.lifecycle.project-delete");
    assert_eq!(cause.sqlite_extended_code, Some(1811));
    assert_eq!(order.lock().unwrap().as_slice(), ["delete"]);
    assert_eq!(fixture.state(), before);
    assert_eq!(fixture.reopen().state(), before);
}
