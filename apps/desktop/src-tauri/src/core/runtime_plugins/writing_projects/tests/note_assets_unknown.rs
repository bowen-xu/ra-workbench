//! Unconfirmed original Deferred settlement retains Core files and the fence.
use super::{
    files::{seed, store},
    *,
};
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{FileRequest, note_assets::Call, operate_files},
};
use rusqlite::hooks::{AuthAction, Authorization, TransactionOperation};
use std::fs;

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_note_asset_unknown_rollback_retains_bytes_and_fences_until_reopen() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", "Original");
    {
        let db = store.connection().unwrap();
        db.execute_batch("create trigger reject_asset after insert on manuscript_files when new.file_kind='other' begin select raise(fail,'synthetic asset failure'); end;").unwrap();
        db.authorizer(Some(
            |context: rusqlite::hooks::AuthContext<'_>| match context.action {
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => Authorization::Deny,
                _ => Authorization::Allow,
            },
        ));
    }
    let error = operate_files(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        f.active.clone(),
        FileRequest::NoteAsset {
            call: Call::Import {
                display_name: "a.txt".into(),
                mime_type: "text/plain".into(),
                bytes: b"retained".to_vec(),
            },
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert!(matches!(error.save_outcome, Some(SaveOutcome::Unknown)));
    assert!(error.save_rollback_error.is_some());
    assert!(error.request_id.is_some());
    assert!(store.connection().is_err());
    let root = crate::scoped_text_files::legacy::manuscript_source_directory(
        store.manuscript_directory(),
        &f.active,
    )
    .unwrap();
    let retained = fs::read_dir(root.join(".ranote-assets"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 1);
    assert_eq!(fs::read(&retained[0]).unwrap(), b"retained");
    f.service.shutdown().await.unwrap();
    drop(store);
    let Fixture {
        directory,
        database,
        ..
    } = f;
    drop(database);
    let db = rusqlite::Connection::open(directory.path().join("synthetic.sqlite3")).unwrap();
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row(
            "select count(*) from manuscript_files where file_kind='other'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(fs::read(&retained[0]).unwrap(), b"retained");
}
