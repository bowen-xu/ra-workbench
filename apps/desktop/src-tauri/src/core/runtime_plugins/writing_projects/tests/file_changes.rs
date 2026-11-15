use super::*;
use super::{
    file_save::{note, request},
    files::{journal, seed, store},
};
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{FileRequest, FileResult, operate_files},
};
use research_workbench_native_sdk::{
    writing_file::FileChangeRequest, writing_project::CitationKeyWrite,
};

async fn change(
    fixture: &Fixture,
    store: &AppStore,
    request: FileChangeRequest,
) -> NativeResult<FileResult> {
    operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        request.manuscript_id().into(),
        FileRequest::Change { request },
    )
    .await
}
fn rename(id: &str, from: &str, to: &str) -> FileChangeRequest {
    FileChangeRequest::Rename {
        manuscript_id: id.into(),
        from_path: from.into(),
        to_path: to.into(),
    }
}
fn delete(id: &str, path: &str) -> FileChangeRequest {
    FileChangeRequest::Delete {
        manuscript_id: id.into(),
        relative_path: path.into(),
    }
}
async fn save_note(fixture: &Fixture, store: &AppStore, path: &str) -> String {
    let content = note("stable-note", "stable search text");
    operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        fixture.active.clone(),
        FileRequest::Save {
            request: request(&fixture.active, path, &content, None),
        },
    )
    .await
    .unwrap();
    content
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_renames_root_and_note_preserving_original_lock_boundary_body_and_identity()
{
    let fixture = Fixture::new();
    let store = store(&fixture);
    let hash = seed(&store, &fixture.active, "main.tex", "root");
    let previous = seed(&store, &fixture.active, "untouched.tex", "old");
    journal(
        &fixture,
        &[CitationKeyWrite {
            path: "untouched.tex".into(),
            previous,
            content: "pending".into(),
        }],
    );
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let locked = store.clone();
    let owner = tokio::task::spawn_blocking(move || {
        let _guard = locked.bibliography_io().unwrap();
        ready.send(()).unwrap();
        let _ = held.recv();
    });
    waiting.await.unwrap();
    let FileResult::Project(result) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        change(
            &fixture,
            &store,
            rename(&format!(" {} ", fixture.active), " main.tex ", "moved.tex"),
        ),
    )
    .await
    .expect("rename must not wait for bibliography")
    .unwrap() else {
        panic!("project")
    };
    assert_eq!(result.id, fixture.active);
    assert_eq!(result.root_path, "moved.tex");
    assert_eq!(result.title, "Legacy title");
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("moved.tex")
            .unwrap(),
        ("root".into(), hash)
    );
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("untouched.tex")
            .unwrap()
            .0,
        "old"
    );
    assert_eq!(
        fixture
            .database
            .lock()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_citation_key_writes",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    release.send(()).unwrap();
    owner.await.unwrap();
    let content = save_note(&fixture, &store, "notes/Old.ranote").await;
    let source = store
        .manuscript_directory()
        .join(&fixture.active)
        .join("source");
    std::fs::File::options()
        .write(true)
        .open(source.join("notes/Old.ranote"))
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
    change(
        &fixture,
        &store,
        rename(&fixture.active, "notes/Old.ranote", "notes/Renamed.RANOTE"),
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("notes/Renamed.RANOTE")
            .unwrap()
            .0,
        content
    );
    assert!(
        std::fs::metadata(source.join("notes/Renamed.RANOTE"))
            .unwrap()
            .modified()
            .unwrap()
            > std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)
    );
    {
        let db = fixture.database.lock().unwrap();
        let row:(String,String,String,String)=db.query_row("select note_id,title,state,search_text from writing_note_catalog join writing_note_catalog_body using(manuscript_id,relative_path) where manuscript_id=?1 and relative_path='notes/Renamed.RANOTE'",[&fixture.active],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        assert_eq!(
            row,
            (
                "stable-note".into(),
                "Renamed".into(),
                "ready".into(),
                "stable search text".into()
            )
        );
    }
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_rename_keeps_physical_and_index_prefix_before_root_validation_or_missing_index()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(&store, &fixture.active, "main.tex", "root");
    let error = change(
        &fixture,
        &store,
        rename(&fixture.active, "main.tex", " moved.tex "),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    assert_eq!(error.message, "稿件路径无效");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("moved.tex")
            .unwrap()
            .0,
        "root"
    );
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select root_path from manuscripts where id=?1",
                [&fixture.active],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "main.tex"
        );
        assert_eq!(
            db.query_row(
                "select relative_path from manuscript_files where manuscript_id=?1",
                [&fixture.active],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "moved.tex"
        );
    }
    store
        .manuscript_files(&fixture.active)
        .save("loose.tex", "loose", None)
        .unwrap();
    let error = change(
        &fixture,
        &store,
        rename(&fixture.active, "loose.tex", "loose-new.tex"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert_eq!(error.message, "稿件文件不存在");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("loose-new.tex")
            .unwrap()
            .0,
        "loose"
    );
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_deletion_protects_root_and_cascades_note_catalog_while_allowing_unindexed_file()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(&store, &fixture.active, "main.tex", "root");
    let root = change(&fixture, &store, delete(&fixture.active, " main.tex "))
        .await
        .unwrap_err();
    assert_eq!(root.code, "invalid_input");
    assert_eq!(root.save_outcome, Some(SaveOutcome::NotStarted));
    let bib = change(&fixture, &store, delete("missing", "refs.BIB"))
        .await
        .unwrap_err();
    assert_eq!(bib.code, "managed_bibliography");
    let missing = change(&fixture, &store, delete("missing", "../bad.tex"))
        .await
        .unwrap_err();
    assert_eq!(missing.code, "not_found");
    assert_eq!(missing.message, "稿件不存在");
    save_note(&fixture, &store, "note.ranote").await;
    assert!(matches!(
        change(&fixture, &store, delete(&fixture.active, "note.ranote"))
            .await
            .unwrap(),
        FileResult::Deleted
    ));
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from writing_note_catalog where manuscript_id=?1",
                [&fixture.active],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(db.query_row("select count(*) from writing_note_catalog_body_search where writing_note_catalog_body_search match 'stable'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    }
    store
        .manuscript_files(&fixture.active)
        .save("unindexed.tex", "raw", None)
        .unwrap();
    assert!(matches!(
        change(&fixture, &store, delete(&fixture.active, "unindexed.tex"))
            .await
            .unwrap(),
        FileResult::Deleted
    ));
    let missing = change(&fixture, &store, delete(&fixture.active, "unindexed.tex"))
        .await
        .unwrap_err();
    assert_eq!(missing.code, "not_found");
    assert_eq!(missing.save_outcome, Some(SaveOutcome::NotStarted));
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_file_change_sql_failures_preserve_original_autocommit_prefixes_and_disk_effects()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(&store, &fixture.active, "a.tex", "A");
    seed(&store, &fixture.active, "b.tex", "B");
    fixture.database.lock().unwrap().execute_batch("create table synthetic_change_prefix(value text);create temp trigger test_file_rename_fail before update on manuscript_files begin insert into synthetic_change_prefix values('rename');select raise(fail,'PRIVATE_RENAME_FAILURE');end;").unwrap();
    let error = change(
        &fixture,
        &store,
        rename(&fixture.active, "a.tex", "renamed.tex"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.file-rename"
    );
    assert!(!error.message.contains("PRIVATE_RENAME_FAILURE"));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("renamed.tex")
            .unwrap()
            .0,
        "A"
    );
    fixture.database.lock().unwrap().execute_batch("drop trigger test_file_rename_fail;create temp trigger test_file_delete_fail before delete on manuscript_files begin insert into synthetic_change_prefix values('delete');select raise(fail,'PRIVATE_DELETE_FAILURE');end;").unwrap();
    let error = change(&fixture, &store, delete(&fixture.active, "b.tex"))
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.file-delete"
    );
    assert!(!error.message.contains("PRIVATE_DELETE_FAILURE"));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(
        !store
            .manuscript_directory()
            .join(&fixture.active)
            .join("source/b.tex")
            .exists()
    );
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row("select count(*) from synthetic_change_prefix", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            2
        );
        assert_eq!(
            db.query_row(
                "select count(*) from manuscript_files where manuscript_id=?1",
                [&fixture.active],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
    }
    fixture.service.shutdown().await.unwrap();
}
