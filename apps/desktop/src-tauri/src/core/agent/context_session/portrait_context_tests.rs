use super::{
    Error,
    sources::{self, Resource},
};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
    params,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

// Isolated read-schema fixtures allow malformed persisted payload types without
// changing product migrations or bypassing production reading/formatting code.
fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table bibliographic_items(id text,title text,item_type text,abstract_text text,issued_year integer,container_title text,volume text,issue text,pages text,url text,language text,bibtex_key text,trashed_at text,archived_at text);
        create table creators(id text,family_name text,given_name text,literal_name text);
        create table item_creators(item_id text,creator_id text,role text,position integer);
        create table identifiers(id text,item_id text,kind text,display_value text);
        create table ai_candidates(id text,candidate_kind text,current_revision integer,review_state text,evidence_bundle_id text,created_at text);
        create table ai_candidate_revisions(candidate_id text,revision integer,payload_json);
        create table ai_evidence_items(bundle_id text,paper_id text);
        insert into bibliographic_items values('paper','Paper','document','',null,'','','','','','','',null,null),('empty-paper','Empty','document','',null,'','','','','','','',null,null);").unwrap();
    db
}
fn candidate(db: &Connection, id: &str, time: &str, papers: &[&str], payload: &str) {
    let bundle = format!("bundle-{id}");
    db.execute(
        "insert into ai_candidates values(?1,'paper_portrait_v1',1,'unreviewed',?2,?3)",
        params![id, bundle, time],
    )
    .unwrap();
    db.execute(
        "insert into ai_candidate_revisions values(?1,1,?2)",
        params![id, payload],
    )
    .unwrap();
    for paper in papers {
        db.execute(
            "insert into ai_evidence_items values(?1,?2)",
            params![bundle, paper],
        )
        .unwrap();
    }
}
fn materialize(
    db: &Connection,
    store: &crate::storage::AppStore,
    paper: &str,
) -> Result<research_agent_core::submission::ContextText, Error> {
    sources::materialize(
        db,
        store,
        &[Resource::PaperPortrait {
            resource_id: paper.into(),
        }],
    )
    .map(|mut rows| rows.remove(0))
}

#[test]
fn portrait_materialize_uses_current_exclusive_revisions_and_the_original_transaction_identity() {
    let db = database();
    candidate(&db, "second", "same", &["paper"], r#"{"second":true}"#);
    candidate(&db, "first", "same", &["paper"], r#"{"obsolete":true}"#);
    candidate(&db, "earliest", "000", &["paper"], "[1,2]");
    db.execute(
        "insert into ai_candidate_revisions values('first',2,?1)",
        [r#"{"current":2,"arbitrary":"JSON"}"#],
    )
    .unwrap();
    db.execute(
        "update ai_candidates set current_revision=2,review_state='rejected' where id='first'",
        [],
    )
    .unwrap();
    db.execute(
        "update ai_candidates set review_state='accepted_after_edit' where id='second'",
        [],
    )
    .unwrap();
    for (id, papers) in [
        ("mixed", vec!["paper", "other"]),
        ("other", vec!["other"]),
        ("unscoped", vec![]),
    ] {
        candidate(&db, id, "001", &papers, "not-json");
    }
    candidate(&db, "other-kind", "001", &["paper"], "not-json");
    db.execute(
        "update ai_candidates set candidate_kind='paper_overview_unit_v1' where id='other-kind'",
        [],
    )
    .unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let store = crate::storage::AppStore::for_test(db, temporary.path().join("objects"));
    let mut db = store.connection().unwrap();
    let changes = db.total_changes();
    let before = materialize(&db, &store, "paper").unwrap();
    let expected=json!({"portraits":[{"reviewState":"unreviewed","payload":[1,2]},{"reviewState":"rejected","payload":{"current":2,"arbitrary":"JSON"}},{"reviewState":"accepted_after_edit","payload":{"second":true}}]}).to_string();
    assert_eq!(before.text, expected);
    assert_eq!(before.source_id, "paper:paper:portrait");
    assert_eq!(before.resource_id, "paper");
    assert!(matches!(
        before.kind,
        research_agent_core::submission::ContextSourceKind::Paper
    ));
    assert_eq!(before.anchor.as_deref(), Some(r#"{"content":"portrait"}"#));
    assert_eq!(
        before.revision,
        format!("sha256:{:x}", Sha256::digest(before.text.as_bytes()))
    );
    let empty = materialize(&db, &store, "empty-paper").unwrap();
    assert_eq!(empty.text, r#"{"portraits":[]}"#);
    assert_eq!(empty.source_id, "paper:empty-paper:portrait");
    assert_eq!(db.total_changes(), changes);
    let tx = db.transaction().unwrap();
    tx.execute(
        "insert into ai_candidate_revisions values('first',3,?1)",
        [r#"{"transaction":"uncommitted Ω"}"#],
    )
    .unwrap();
    tx.execute(
        "update ai_candidates set current_revision=3,review_state='edited' where id='first'",
        [],
    )
    .unwrap();
    tx.execute(
        "update bibliographic_items set archived_at='now' where id='paper'",
        [],
    )
    .unwrap();
    let changes = tx.total_changes();
    let current = materialize(&tx, &store, "paper").unwrap();
    let expected=json!({"portraits":[{"reviewState":"unreviewed","payload":[1,2]},{"reviewState":"edited","payload":{"transaction":"uncommitted Ω"}},{"reviewState":"accepted_after_edit","payload":{"second":true}}]}).to_string();
    assert_eq!(current.text, expected);
    assert_eq!(current.source_id, before.source_id);
    assert_eq!(current.resource_id, before.resource_id);
    assert_eq!(current.anchor, before.anchor);
    assert_eq!(
        current.revision,
        format!("sha256:{:x}", Sha256::digest(current.text.as_bytes()))
    );
    assert_ne!(current.revision, before.revision);
    assert_eq!(tx.total_changes(), changes);
    tx.rollback().unwrap();
    let restored = materialize(&db, &store, "paper").unwrap();
    assert_eq!(restored.text, before.text);
    assert_eq!(restored.revision, before.revision);
    assert!(db.is_autocommit());
}

#[test]
fn portrait_materialize_preserves_metadata_sql_row_and_json_error_stages() {
    let temporary = tempfile::tempdir().unwrap();
    let store = crate::storage::AppStore::for_test(database(), temporary.path().join("objects"));
    let db = store.connection().unwrap();
    let body_read = Arc::new(AtomicBool::new(false));
    let late_metadata_error = Arc::new(AtomicBool::new(false));
    let observed = body_read.clone();
    let deny_late = late_metadata_error.clone();
    db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "ai_candidates",
            ..
        } => {
            observed.store(true, Ordering::SeqCst);
            Authorization::Deny
        }
        AuthAction::Read {
            table_name: "identifiers",
            ..
        } if deny_late.load(Ordering::SeqCst) => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    assert_eq!(
        materialize(&db, &store, "missing").err().unwrap(),
        Error::SourceUnavailable
    );
    assert!(!body_read.load(Ordering::SeqCst));
    assert_eq!(
        materialize(&db, &store, "paper").err().unwrap(),
        Error::Unavailable
    );
    assert!(body_read.load(Ordering::SeqCst));
    body_read.store(false, Ordering::SeqCst);
    late_metadata_error.store(true, Ordering::SeqCst);
    assert_eq!(
        materialize(&db, &store, "paper").err().unwrap(),
        Error::Unavailable
    );
    assert!(!body_read.load(Ordering::SeqCst)); // The complete metadata preflight still wins.
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    candidate(&db, "bad-json", "a", &["paper"], "not-json");
    let changes = db.total_changes();
    assert_eq!(
        materialize(&db, &store, "paper").err().unwrap(),
        Error::SourceUnavailable
    );
    assert_eq!(db.total_changes(), changes);
    candidate(&db, "bad-row", "b", &["paper"], "placeholder");
    db.execute(
        "update ai_candidate_revisions set payload_json=?1 where candidate_id='bad-row'",
        [rusqlite::types::Value::Blob(vec![1, 2])],
    )
    .unwrap();
    let changes = db.total_changes();
    // The later row's String decode fails before the earlier invalid JSON can
    // be parsed: the original query first collects every raw row.
    assert_eq!(
        materialize(&db, &store, "paper").err().unwrap(),
        Error::Unavailable
    );
    assert_eq!(db.total_changes(), changes);
    assert!(db.is_autocommit());
}
