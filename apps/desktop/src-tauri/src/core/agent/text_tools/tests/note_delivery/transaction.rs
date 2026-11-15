//! Actual SQLite failures distinguish verified rollback from uncertain commit/cleanup.
use super::*;

fn fail_terminal(f: &Fixture, action: &str) {
    assert!(matches!(action, "abort" | "rollback"));
    f.tools
        .store
        .connection()
        .unwrap()
        .execute_batch(&format!(
            "create temp trigger reject_note_receipt before update on main.agent_text_patch_calls
         when new.state='complete' and new.request_id like '%:notes'
         begin select raise({action},'private synthetic SQL cause'); end;"
        ))
        .unwrap();
}
async fn journal_unknown(f: &Fixture, id: &str, phase: &str) -> Value {
    let result = f.tools.authorize_and_execute(call(f, id)).await;
    let Err(RunError::ToolOutcomeUnknownDiagnostic(detail)) = result else {
        panic!("uncertain original note transaction must require recovery");
    };
    let value = serde_json::to_value(detail).unwrap();
    assert_eq!(value["code"], "TEXT_PATCH_JOURNAL_UNCONFIRMED");
    assert_eq!(value["phase"], phase);
    assert_eq!(value["targetRequestId"], format!("{}:notes", key(f, id)));
    assert!(!value.to_string().contains("private synthetic"));
    value
}
fn defer_commit_failure(f: &Fixture) {
    f.tools.store.connection().unwrap().execute_batch(
        "create temp table commit_parent(id integer primary key);
         create temp table commit_child(parent integer references commit_parent(id) deferrable initially deferred);
         create temp trigger reject_note_commit after update on main.agent_text_patch_calls
         when new.state='complete' and new.request_id like '%:notes'
         begin insert into commit_child(parent) values(7); end;"
    ).unwrap();
}
#[tokio::test]
async fn refused_terminal_receipt_rolls_back_source_baseline_and_snapshots_before_definite_feedback()
 {
    let f = fixture(true);
    read(&f, &path(f.page)).await;
    let original = source(&f);
    fail_terminal(&f, "abort");
    let result = f
        .tools
        .authorize_and_execute(call(&f, "rollback-ok"))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        !serde_json::to_string(&result.content)
            .unwrap()
            .contains("private synthetic")
    );
    assert_eq!(source(&f), original);
    let db = f.tools.store.connection().unwrap();
    assert!(db.is_autocommit());
    for table in ["agent_note_patch_candidates", "agent_note_patch_snapshots"] {
        let count: i64 = db
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
    assert_eq!(
        db.query_row::<i64, _, _>(
            "select count(*) from agent_text_patch_calls where request_id=?1",
            [format!("{}:notes", key(&f, "rollback-ok"))],
            |row| row.get(0)
        )
        .unwrap(),
        0
    );
}
#[tokio::test]
async fn actual_commit_constraint_failure_is_unknown_with_original_driver_evidence() {
    let f = fixture(true);
    read(&f, &path(f.page)).await;
    let original = source(&f);
    defer_commit_failure(&f);
    let detail = journal_unknown(&f, "commit-unknown", "note-commit").await;
    assert_eq!(detail["sqliteExtendedCode"], 787);
    assert_eq!(source(&f), original);
    assert!(f.tools.store.connection().unwrap().is_autocommit());
    forbid_rewrite(&f);
    // Missing original target proof is not permission to reapply a failed commit.
    journal_unknown(&f, "commit-unknown", "note-commit").await;
    assert_eq!(source(&f), original);
}
#[tokio::test]
async fn actual_rollback_error_does_not_become_ordinary_tool_failure() {
    let f = fixture(true);
    read(&f, &path(f.page)).await;
    let original = source(&f);
    fail_terminal(&f, "rollback");
    let detail = journal_unknown(&f, "rollback-unknown", "note-rollback").await;
    assert_eq!(detail["sqliteExtendedCode"], 1);
    assert_eq!(source(&f), original);
    assert!(f.tools.store.connection().unwrap().is_autocommit());
}
#[tokio::test]
async fn uncertain_transaction_blocks_the_actual_log_owner() {
    let (f, _events) = with_events();
    read(&f, &path(f.page)).await;
    defer_commit_failure(&f);
    let detail = journal_unknown(&f, "blocked-owner", "note-commit").await;
    assert_eq!(detail["sqliteExtendedCode"], 787);
    assert!(f.tools.store.connection().is_err());
}
async fn changes(f: &Fixture) -> Vec<TextChange> {
    let snapshot = f
        .tools
        .backend(&path(f.page))
        .unwrap()
        .read(&path(f.page), false, true)
        .await
        .unwrap();
    let hunks = text_patch::parse(&patch(&path(f.page), "old", "new")).unwrap();
    vec![research_agent_core::text_patch::backend::prepare(&hunks[0], Some(snapshot)).unwrap()]
}
#[tokio::test]
async fn original_running_intent_is_checked_before_stale_note_preparation_and_reused_id_conflicts()
{
    let f = fixture(true);
    let changes = changes(&f).await;
    let input = serde_json::to_string(&changes).unwrap();
    let target = format!("{}:notes", key(&f, "running"));
    assert!(
        journal::insert_sql(
            &f.tools.store.connection().unwrap(),
            f.tools.record.conversation_id,
            &target,
            &input,
        )
        .is_ok()
    );
    let later = later_input(&f);
    forbid_rewrite(&f);
    let backend = f.tools.backend(&path(f.page)).unwrap();
    let result = backend.commit(&target, changes.clone()).await.unwrap();
    assert_eq!(result[0].status, TargetStatus::Unknown);
    let mut changed = changes;
    changed[0].after = Some("another patch".into());
    let error = backend.commit(&target, changed).await.unwrap_err();
    assert!(error.contains("TEXT_PATCH_IDENTITY_CONFLICT"));
    assert_eq!(source(&f), later);
}
#[tokio::test]
async fn uncommitted_rows_cannot_supply_note_query_or_delivery_proof() {
    let f = fixture(true);
    read(&f, &path(f.page)).await;
    f.tools
        .authorize_and_execute(call(&f, "committed"))
        .await
        .unwrap();
    let target = format!("{}:notes", key(&f, "committed"));
    let db = f.tools.store.connection().unwrap();
    db.execute_batch("begin;").unwrap();
    assert!(matches!(
        note::delivery::read(&db, &f.tools.record, &target),
        Err(note::delivery::Error::Transaction)
    ));
    drop(db);
    let error = f
        .tools
        .backend(&path(f.page))
        .unwrap()
        .query(&target)
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::from_str::<Value>(&error).unwrap()["safeCause"],
        "note_transaction_unconfirmed"
    );
    f.tools
        .store
        .connection()
        .unwrap()
        .execute_batch("rollback;")
        .unwrap();
}
