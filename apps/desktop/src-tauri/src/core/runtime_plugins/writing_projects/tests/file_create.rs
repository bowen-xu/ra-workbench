use super::*;
use super::{
    file_save::{note, request},
    files::{journal, seed, store},
};
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{FileRequest, FileResult, operate_files},
};
use research_workbench_native_sdk::writing_project::{CitationKeyWrite, SaveFileRequest};

async fn create(
    fixture: &Fixture,
    store: &AppStore,
    request: SaveFileRequest,
) -> NativeResult<FileResult> {
    operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        request.manuscript_id.clone(),
        FileRequest::Create { request },
    )
    .await
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_creates_note_and_rejects_overwrite_without_recovering_unrelated_pending_writes()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let previous = seed(&store, &fixture.active, "main.tex", "old");
    journal(
        &fixture,
        &[CitationKeyWrite {
            path: "main.tex".into(),
            previous,
            content: "pending".into(),
        }],
    );
    // The original create command does not acquire the bibliography lock.
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let locked_store = store.clone();
    let lock_owner = tokio::task::spawn_blocking(move || {
        let _guard = locked_store.bibliography_io().unwrap();
        ready.send(()).unwrap();
        let _ = held.recv();
    });
    waiting.await.unwrap();
    let content = note("created-note", "created text");
    let FileResult::Read(file) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        create(
            &fixture,
            &store,
            request(
                &format!(" {} ", fixture.active),
                " notes/创建.ranote ",
                &content,
                None,
            ),
        ),
    )
    .await
    .expect("create must not wait for the original bibliography owner")
    .unwrap() else {
        panic!("created")
    };
    assert_eq!(file.relative_path, " notes/创建.ranote ");
    assert_eq!(file.content, content);
    let conflict = create(
        &fixture,
        &store,
        request(&fixture.active, "notes/创建.ranote", "overwrite", None),
    )
    .await
    .unwrap_err();
    assert_eq!(conflict.code, "file_conflict");
    assert_eq!(conflict.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("notes/创建.ranote")
            .unwrap()
            .0,
        content
    );
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .unwrap()
            .0,
        "old"
    );
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from manuscript_citation_key_writes",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(db.query_row("select state from writing_note_catalog where manuscript_id=?1 and relative_path='notes/创建.ranote'",[&fixture.active],|row|row.get::<_,String>(0)).unwrap(),"ready");
    }
    release.send(()).unwrap();
    lock_owner.await.unwrap();
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_creation_index_failure_keeps_created_bytes_and_original_io_context() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    fixture.database.lock().unwrap().execute_batch("create temp trigger test_create_index_fail before insert on manuscript_files begin select raise(fail,'PRIVATE_CREATE_FILE');end;").unwrap();
    let error = create(
        &fixture,
        &store,
        request(&fixture.active, "new.tex", "created", None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.file-upsert"
    );
    assert!(!error.message.contains("PRIVATE_CREATE_FILE"));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("new.tex")
            .unwrap()
            .0,
        "created"
    );
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from manuscript_files where manuscript_id=?1",
                [&fixture.active],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger test_create_index_fail")
            .unwrap();
    }
    let source = store
        .manuscript_directory()
        .join(&fixture.active)
        .join("source");
    std::fs::write(source.join("blocked"), "regular file").unwrap();
    let io = create(
        &fixture,
        &store,
        request(&fixture.active, "blocked/new.tex", "x", None),
    )
    .await
    .unwrap_err();
    assert_eq!(io.code, "storage_error");
    assert!(io.message.starts_with("稿件文件操作失败："));
    assert!(io.instance_id.is_some() && io.request_id.is_some());
    fixture.service.shutdown().await.unwrap();
}
