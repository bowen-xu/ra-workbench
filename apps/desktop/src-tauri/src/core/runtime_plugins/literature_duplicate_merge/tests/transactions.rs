use super::*;
use crate::runtime_plugins::literature_duplicates::tests::fixture::PAGE;

#[test]
fn insert_conflict_uses_actual_target_and_preserves_origin_and_note() {
    let f = Fixture::new();
    f.data.db.lock().unwrap().execute_batch("update nal_state set legacy_imported=1,relation_semantics_version=1;
        insert into nal_statements(id,subject,predicate,origin) values
        ('existing','{\"kind\":\"paper\",\"id\":\"survivor\"}','{\"kind\":\"concept\",\"id\":\"same\"}','human'),
        ('source-fact','{\"kind\":\"paper\",\"id\":\"source\"}','{\"kind\":\"concept\",\"id\":\"same\"}','legacy');").unwrap();
    f.data.db.lock().unwrap().execute("insert into nal_annotations(id,statement_id,note_page_id) values('source-note','source-fact',?1)",[PAGE]).unwrap();
    let note = f.data.note();
    f.apply().unwrap();
    assert_eq!(
        f.scalar::<String>("select origin from nal_statements where id='existing'"),
        "human"
    );
    assert_eq!(f.scalar::<i64>("select count(*) from nal_statements"), 2);
    assert_eq!(
        f.scalar::<i64>("select count(*) from nal_annotations where statement_id='existing'"),
        1
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from nal_annotations where statement_id='source-fact'"),
        1
    );
    assert_eq!(f.data.note(), note);
}

#[test]
fn global_bad_fact_is_decoded_before_annotation_or_rule_reads() {
    let f = Fixture::new();
    f.data.db.lock().unwrap().execute_batch("update nal_state set legacy_imported=1,relation_semantics_version=1;
        insert into nal_statements(id,subject,predicate,origin) values('bad',
        '{\"kind\":\"variable\",\"name\":\"unrelated\"}','{\"kind\":\"concept\",\"id\":\"other\"}','human');").unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = reads.clone();
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Read { table_name, .. }
                if table_name == "nal_annotations" || table_name == "nal_relation_rules" =>
            {
                observed.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }));
    let error = f.apply().unwrap_err();
    assert_eq!(error.outcome, MergeOutcome::RolledBack);
    assert_eq!(error.problem.code(), "storage_error");
    assert!(error.to_string().contains("关系数据结构无效"));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(
        f.scalar::<i64>("select count(*) from bibliographic_items where archived_at is not null"),
        0
    );
}

#[test]
fn denied_commit_rolls_back_and_keeps_the_owner_usable() {
    let f = Fixture::new();
    f.collection();
    let note = f.data.note();
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(|ctx: AuthContext<'_>| {
            if matches!(
                ctx.action,
                AuthAction::Transaction {
                    // Pinned rusqlite 0.37 reports COMMIT as Unknown, as in
                    // the existing guarded-transaction/undo fault fixtures.
                    operation: TransactionOperation::Unknown
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let error = f.apply().unwrap_err();
    assert_eq!(error.outcome, MergeOutcome::RolledBack);
    assert!(error.operation_id.is_some());
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(f.scalar::<i64>("select count(*) from merge_operations"), 0);
    assert_eq!(f.scalar::<i64>("select count(*) from concepts"), 0);
    assert_eq!(f.data.note(), note);
    f.apply().unwrap();
}

#[test]
fn denied_rollback_fences_the_original_owner_until_full_cold_reopen() {
    let f = Fixture::new();
    f.collection();
    let note = f.data.note();
    f.data.db.lock().unwrap().execute_batch("create trigger fail_history before insert on merge_operations begin select raise(abort,'synthetic history failure'); end;").unwrap();
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(|ctx: AuthContext<'_>| {
            if matches!(
                ctx.action,
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    let error = f.apply().unwrap_err();
    assert_eq!(error.outcome, MergeOutcome::Unknown);
    assert!(error.rollback_error.is_some());
    assert!(error.operation_id.is_some());
    assert!(f.data.db.lock().is_err());
    let f = f.reopen_fenced();
    assert_eq!(f.scalar::<i64>("select count(*) from merge_operations"), 0);
    assert_eq!(f.scalar::<i64>("select count(*) from concepts"), 0);
    assert_eq!(
        f.scalar::<i64>("select count(*) from bibliographic_items where archived_at is not null"),
        0
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from collection_memberships where item_id='survivor'"),
        0
    );
    assert_eq!(f.data.note(), note);
}
