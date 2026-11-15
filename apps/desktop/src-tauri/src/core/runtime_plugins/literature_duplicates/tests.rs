use super::*;
use research_workbench_native_sdk::{
    data::DataValue, duplicate_review::DuplicateDecisionRequestV1,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) mod fixture;
use fixture::Fixture;

#[test]
fn fixed_reads_retain_original_order_lossless_bits_and_no_begin() {
    let fixture = Fixture::new();
    fixture.candidate("next-down-one", f64::from_bits(0x3fefffffffffffff));
    fixture.candidate("one", 1.0);
    let body = fixture.note();
    let begins = Arc::new(AtomicUsize::new(0));
    {
        let db = fixture.db.lock().unwrap();
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
        let mut rows = fixture.resources.candidates(&db).unwrap();
        assert_eq!(rows.rows.len(), 2);
        assert_eq!(rows.rows[0][0], DataValue::Text("one".into()));
        assert_eq!(rows.rows[1][0], DataValue::Text("next-down-one".into()));
        assert_eq!(rows.rows[1][6], DataValue::Text("3fefffffffffffff".into()));
        let original = rows.clone();
        for _ in 0..8 {
            rows = serde_json::from_str(&serde_json::to_string(&rows).unwrap()).unwrap();
            assert_eq!(rows, original);
        }
        db.execute_batch("update bibliographic_items set archived_at='2026' where id='source';
            insert into merge_operations(id,survivor_item_id,merged_item_id,snapshot_json,state,created_at)
            values('history','survivor','source','{}','applied','2026');").unwrap();
        assert!(fixture.resources.candidates(&db).unwrap().rows.is_empty());
        let rows = fixture.resources.history(&db).unwrap();
        assert_eq!(rows.rows.len(), 1, "history retains an archived endpoint");
        assert_eq!(rows.rows[0][0], DataValue::Text("history".into()));
        assert_eq!(begins.load(Ordering::SeqCst), 0);
        db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
    assert_eq!(fixture.note(), body);
}

#[test]
fn actual_autocommit_success_ignore_and_fail_survive_sql_log_reopen_without_replay() {
    for mode in ["normal", "ignore", "fail"] {
        let fixture = Fixture::new();
        fixture.candidate("candidate", 0.72);
        let note = fixture.note();
        {
            let db = fixture.db.lock().unwrap();
            db.execute_batch("create table synthetic_duplicate_effects(value text not null);")
                .unwrap();
            if mode != "normal" {
                // Fixed synthetic mode strings, never product/user SQL input.
                let raise = if mode == "ignore" {
                    "raise(ignore)"
                } else {
                    "raise(fail,'private synthetic trigger text')"
                };
                db.execute_batch(&format!("create trigger synthetic_duplicate_effect before update of decision on duplicate_candidates begin
                    insert into synthetic_duplicate_effects(value) values('synthetic-effect');
                    select {raise};end;" )).unwrap();
            }
        }
        let request = DuplicateDecisionRequestV1 {
            candidate_id: "candidate".into(),
            decision: "dismiss".into(),
        };
        let result = execute_decision(
            &mut fixture.db.lock().unwrap(),
            &fixture.resources,
            &request,
        );
        match mode {
            "normal" => assert_eq!(result.unwrap(), 1),
            "ignore" => assert_eq!(result.unwrap(), 0),
            _ => {
                let error = result.unwrap_err();
                assert_eq!(error.code, "storage_error");
                assert_eq!(
                    error.duplicate_sql,
                    Some(DecisionSqlState::Failed {
                        observed_autocommit: true
                    })
                );
                assert_eq!(
                    error.data_error.as_ref().unwrap().sqlite_extended_code,
                    Some(1811)
                );
                assert!(
                    !serde_json::to_string(&error)
                        .unwrap()
                        .contains("private synthetic trigger text")
                );
            }
        }
        assert!(fixture.db.lock().unwrap().is_autocommit());
        assert_eq!(fixture.note(), note);
        let fixture = fixture.reopen();
        let actual: (Option<String>, i64) = fixture.db.lock().unwrap().query_row(
            "select (select decision from duplicate_candidates where id='candidate'), (select count(*) from synthetic_duplicate_effects)", [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(
            actual,
            (
                if mode == "normal" {
                    Some("dismiss".into())
                } else {
                    None
                },
                i64::from(mode != "normal")
            )
        );
        assert_eq!(fixture.note(), note);
    }
}
