use super::*;
use crate::agent::{WritingContextError, WritingContextRequest};

#[test]
fn deliberately_missing_source_is_unavailable_without_domain_fallback() {
    let root = tempfile::tempdir().unwrap();
    let mut store = AppStore::for_test(
        Connection::open_in_memory().unwrap(),
        root.path().join("objects"),
    );
    store.writing_context_source = None;
    let db = store.connection().unwrap();
    assert!(matches!(
        store.materialize_writing_context_in(
            &db,
            WritingContextRequest {
                manuscript_id: uuid::Uuid::nil(),
                files: vec![],
            },
        ),
        Err(WritingContextError::Unavailable)
    ));
    assert!(matches!(
        store.materialize_current_writing_file_in(&db, uuid::Uuid::nil(), "main.tex"),
        Err(WritingContextError::Unavailable)
    ));
}
