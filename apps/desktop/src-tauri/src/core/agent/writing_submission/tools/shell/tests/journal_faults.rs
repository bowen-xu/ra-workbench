//! Real filesystem effects and real SQLite refusal; recovery never reopens a writer.
use super::*;
use crate::agent::text_tools::file_workspace::FileWorkspace;
use rusqlite::{OptionalExtension, params};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[path = "journal_faults/query.rs"]
mod query;
#[path = "journal_faults/zero_rows.rs"]
mod zero_rows;

#[derive(Default)]
struct Access {
    calls: AtomicUsize,
    blocked: AtomicBool,
}
struct ObservedSource {
    inner: Arc<dyn FileWorkspaceSource>,
    access: Arc<Access>,
}
impl FileWorkspaceSource for ObservedSource {
    fn workspace<'a>(&'a self, record: &'a SubmissionRecord) -> RunFuture<'a, FileWorkspace> {
        Box::pin(async move {
            self.access.calls.fetch_add(1, Ordering::SeqCst);
            if self.access.blocked.load(Ordering::SeqCst) {
                return Err(RunError::ToolRejected);
            }
            self.inner.workspace(record).await
        })
    }
}
async fn observed() -> (Harness, Arc<Access>) {
    let access = Arc::new(Access::default());
    let probe = access.clone();
    let h = Harness::with_source(move |inner| {
        Arc::new(ObservedSource {
            inner,
            access: probe,
        })
    })
    .await;
    (h, access)
}
fn key(h: &Harness, id: &str) -> String {
    format!("{}:{id}", h.record.request_id)
}
fn row(h: &Harness, key: &str) -> Option<(String, Value)> {
    let db = h.fixture.store.connection().unwrap();
    let held:bool=db.query_row("select exists(select 1 from sqlite_schema where type='table' and name='held_patch_calls')",[],|row|row.get(0)).unwrap();
    let query = if held {
        "select state,results_json from main.held_patch_calls where conversation_id=?1 and request_id=?2"
    } else {
        "select state,results_json from main.agent_text_patch_calls where conversation_id=?1 and request_id=?2"
    };
    let result: Option<(String, String)> = db
        .query_row(
            query,
            params![h.record.conversation_id.to_string(), key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap();
    result.map(|(state, value)| (state, serde_json::from_str(&value).unwrap()))
}
fn reject(h: &Harness, key: &str, point: &str) {
    let db = h.fixture.store.connection().unwrap();
    db.execute_batch("create temp table patch_journal_fault(request_id text primary key,point text not null);
        create temp trigger patch_journal_insert_fault before insert on main.agent_text_patch_calls
        when exists(select 1 from patch_journal_fault f where f.request_id=new.request_id and f.point='insert')
        begin select raise(abort,'private synthetic journal cause'); end;
        create temp trigger patch_journal_update_fault before update on main.agent_text_patch_calls
        when exists(select 1 from patch_journal_fault f where f.request_id=old.request_id and (f.point='update' or (f.point='terminal' and new.state='complete')))
        begin select raise(abort,'private synthetic journal cause'); end;
        create temp trigger patch_journal_update_ignore before update on main.agent_text_patch_calls
        when exists(select 1 from patch_journal_fault f where f.request_id=old.request_id and f.point='ignore-terminal' and new.state='complete')
        begin select raise(ignore); end;
        create temp trigger patch_journal_insert_ignore before insert on main.agent_text_patch_calls
        when exists(select 1 from patch_journal_fault f where f.request_id=new.request_id and f.point='ignore-insert')
        begin select raise(ignore); end;").unwrap();
    db.execute(
        "insert into patch_journal_fault(request_id,point) values(?1,?2)",
        params![key, point],
    )
    .unwrap();
}
fn clear(h: &Harness) {
    h.fixture.store.connection().unwrap().execute_batch("drop trigger patch_journal_insert_fault;drop trigger patch_journal_update_fault;drop trigger patch_journal_update_ignore;drop trigger patch_journal_insert_ignore;drop table patch_journal_fault;").unwrap();
}
fn seal_probe(access: &Access) -> usize {
    access.blocked.store(true, Ordering::SeqCst);
    access.calls.load(Ordering::SeqCst)
}
async fn unknown(h: &Harness, call: ToolCall) -> Value {
    match h.tools.authorize_and_execute(call).await {
        Err(RunError::ToolOutcomeUnknown) => Value::Null,
        Err(RunError::ToolOutcomeUnknownDiagnostic(detail)) => {
            let detail = serde_json::to_value(detail).unwrap();
            assert_eq!(detail["code"], "TEXT_PATCH_JOURNAL_UNCONFIRMED");
            assert_eq!(detail["conversationId"], json!(h.record.conversation_id));
            assert_eq!(detail["requestId"], json!(h.record.request_id));
            assert_eq!(detail["turnId"], json!(h.record.turn_id));
            assert_eq!(
                detail["contextSnapshotId"],
                json!(h.record.context_snapshot_id)
            );
            assert!(!detail.to_string().contains("private synthetic"));
            assert!(!detail.to_string().contains("Frozen manuscript"));
            detail
        }
        _ => panic!("started effects require recovery, never ordinary feedback"),
    }
}
async fn sql_unknown(h: &Harness, call: ToolCall, phase: &str, extended: Option<i32>) -> Value {
    let detail = unknown(h, call).await;
    assert_eq!(detail["phase"], phase);
    assert_eq!(detail["sqliteExtendedCode"], json!(extended));
    detail
}
fn diagnostic(h: &Harness, id: &str, detail: &Value, phase: &str) {
    assert_eq!(detail["code"], "TEXT_PATCH_JOURNAL_UNCONFIRMED");
    assert_eq!(detail["conversationId"], json!(h.record.conversation_id));
    assert_eq!(detail["requestId"], json!(h.record.request_id));
    assert_eq!(detail["callId"], id);
    assert_eq!(detail["callRequestId"], key(h, id));
    assert_eq!(detail["phase"], phase);
    assert_eq!(detail["sqliteCode"], "ConstraintViolation");
    assert_eq!(detail["sqliteExtendedCode"], 1811);
    assert!(!detail.to_string().contains("private synthetic"));
    assert!(detail.get("epoch").is_none());
    assert!(detail.get("fence").is_none());
}

#[path = "journal_faults/legacy.rs"]
mod legacy;
#[path = "journal_faults/outer.rs"]
mod outer;
#[path = "journal_faults/targets.rs"]
mod targets;
