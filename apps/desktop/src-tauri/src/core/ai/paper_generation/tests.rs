use super::*;
use crate::docling::preparation::test_support::{Parser, attachment};
use rusqlite::Connection;
use std::sync::atomic::Ordering;

fn fixture() -> (tempfile::TempDir, AppStore, std::sync::Arc<Parser>) {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','document','Paper','now','now')", []).unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    attachment(&store, "paper", "file", &"a".repeat(64));
    let parser = Parser::new();
    parser.install(&store);
    (root, store, parser)
}

#[tokio::test]
async fn both_stages_prepare_before_admission_and_reuse_the_same_parse() {
    let (_root, store, parser) = fixture();
    let files = vec!["file".to_owned()];
    let preparation = prepare_sources(&store, "paper", &files);
    let release = async {
        parser.wait_started().await;
        let db = store.connection().unwrap();
        assert_eq!(
            db.query_row("select count(*) from agent_conversations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(db);
        parser.release.add_permits(1);
    };
    let (result, ()) = tokio::join!(preparation, release);
    result.unwrap();
    prepare_sources(&store, "paper", &files).await.unwrap();
    assert_eq!(parser.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn parse_failure_does_not_create_a_conversation_or_run() {
    let (_root, store, parser) = fixture();
    parser.fail.store(true, Ordering::SeqCst);
    parser.release.add_permits(1);
    let result = prepare_sources(&store, "paper", &["file".into()]).await;
    assert!(result.unwrap_err().to_string().contains("file"));
    let db = store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_conversations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from ai_runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
