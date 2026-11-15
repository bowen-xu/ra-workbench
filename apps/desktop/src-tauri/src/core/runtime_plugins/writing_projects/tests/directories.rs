use super::*;
use super::{
    file_save::{note, request},
    files::{seed, store},
};
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{FileRequest, FileResult, directories::DirectoryCall, operate_files},
};
use research_workbench_native_sdk::writing_directory::DirectoryRequest;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

pub(super) async fn directory(
    fixture: &Fixture,
    store: &AppStore,
    call: DirectoryCall,
) -> NativeResult<FileResult> {
    operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        fixture.active.clone(),
        FileRequest::Directory { call },
    )
    .await
}
pub(super) fn rename(id: &str, from: &str, to: &str) -> DirectoryCall {
    DirectoryCall::Change {
        request: DirectoryRequest::Rename {
            manuscript_id: id.into(),
            from_path: from.into(),
            to_path: to.into(),
        },
    }
}
fn delete(id: &str, path: &str) -> DirectoryCall {
    DirectoryCall::Change {
        request: DirectoryRequest::Delete {
            manuscript_id: id.into(),
            relative_path: path.into(),
        },
    }
}
pub(super) fn source(store: &AppStore, id: &str) -> std::path::PathBuf {
    store.manuscript_directory().join(id).join("source")
}
fn session(fixture: &Fixture, open: &str, active: &str, cursor: &str) {
    fixture.database.lock().unwrap().execute(r#"insert into writing_sessions(manuscript_id,open_paths_json,active_path,cursor_state_json,layout_state_json,updated_at) values(?1,?2,?3,?4,'{"preview":false}','old')"#,rusqlite::params![fixture.active,open,active,cursor]).unwrap();
}
fn draft(fixture: &Fixture, path: &str) {
    fixture
        .database
        .lock()
        .unwrap()
        .execute(
            "insert into manuscript_recovery_drafts values(?1,?2,?3,'unsaved','old')",
            rusqlite::params![fixture.active, path, "a".repeat(64)],
        )
        .unwrap();
}
fn staged(store: &AppStore, id: &str) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(store.manuscript_directory().join(id))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".writing-delete-")
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_directory_list_create_preserves_sort_skip_symlink_and_conflict() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(&store, &fixture.active, "main.tex", "root");
    let FileResult::Directory(created) = directory(
        &fixture,
        &store,
        DirectoryCall::Create {
            relative_path: " z/子目录 ".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("directory")
    };
    assert_eq!(created.relative_path, "z/子目录");
    directory(
        &fixture,
        &store,
        DirectoryCall::Create {
            relative_path: "A".into(),
        },
    )
    .await
    .unwrap();
    std::fs::create_dir_all(source(&store, &fixture.active).join(".agent-before-hidden/ignored"))
        .unwrap();
    let FileResult::Directories(list) = directory(&fixture, &store, DirectoryCall::List)
        .await
        .unwrap()
    else {
        panic!("list")
    };
    assert_eq!(
        list.iter()
            .map(|item| item.relative_path.as_str())
            .collect::<Vec<_>>(),
        ["A", "z", "z/子目录"]
    );
    let error = directory(
        &fixture,
        &store,
        DirectoryCall::Create {
            relative_path: "z".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "file_conflict");
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    let error = directory(
        &fixture,
        &store,
        DirectoryCall::Create {
            relative_path: "../bad".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "invalid_input");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            fixture.directory.path(),
            source(&store, &fixture.active).join("linked"),
        )
        .unwrap();
        let error = directory(&fixture, &store, DirectoryCall::List)
            .await
            .unwrap_err();
        assert_eq!(error.code, "unsafe_path");
        assert!(error.message.contains("符号链接"));
    }
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_directory_rename_updates_original_root_session_recovery_and_note_relations()
{
    let fixture = Fixture::new();
    let store = store(&fixture);
    let hash = seed(&store, &fixture.active, "chapters/main.tex", "root");
    seed(
        &store,
        &fixture.active,
        "chapters-old/keep.tex",
        "unchanged",
    );
    let content = note("stable-folder-note", "folderrare search body");
    operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        fixture.active.clone(),
        FileRequest::Save {
            request: request(&fixture.active, "chapters/note.ranote", &content, None),
        },
    )
    .await
    .unwrap();
    fixture
        .database
        .lock()
        .unwrap()
        .execute(
            "update manuscripts set root_path='chapters/main.tex' where id=?1",
            [&fixture.active],
        )
        .unwrap();
    session(
        &fixture,
        r#"["chapters/main.tex","chapters-old/keep.tex"]"#,
        "chapters/note.ranote",
        r#"{"chapters/main.tex":{"head":8},"chapters-old/keep.tex":1}"#,
    );
    draft(&fixture, "chapters/main.tex");
    draft(&fixture, "chapters-old/keep.tex");
    let FileResult::Project(result) = directory(
        &fixture,
        &store,
        rename(&fixture.active, "chapters", "renamed"),
    )
    .await
    .unwrap() else {
        panic!("project")
    };
    assert_eq!(result.root_path, "renamed/main.tex");
    assert_eq!(result.title, "Legacy title");
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("renamed/main.tex")
            .unwrap(),
        ("root".into(), hash)
    );
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("renamed/note.ranote")
            .unwrap()
            .0,
        content
    );
    assert!(!source(&store, &fixture.active).join("chapters").exists());
    {
        let db = fixture.database.lock().unwrap();
        let row:(String,String,String,String)=db.query_row("select open_paths_json,active_path,cursor_state_json,layout_state_json from writing_sessions where manuscript_id=?1",[&fixture.active],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.0).unwrap(),
            serde_json::json!(["renamed/main.tex", "chapters-old/keep.tex"])
        );
        assert_eq!(row.1, "renamed/note.ranote");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.2).unwrap(),
            serde_json::json!({"renamed/main.tex":{"head":8},"chapters-old/keep.tex":1})
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.3).unwrap(),
            serde_json::json!({"preview":false})
        );
        let paths = db
            .prepare("select relative_path from manuscript_recovery_drafts order by relative_path")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(paths, ["chapters-old/keep.tex", "renamed/main.tex"]);
        let row: (String, String) = db
            .query_row(
                "select note_id,relative_path from writing_note_catalog where manuscript_id=?1",
                [&fixture.active],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            ("stable-folder-note".into(), "renamed/note.ranote".into())
        );
        assert_eq!(db.query_row("select count(*) from writing_note_catalog_body_search where writing_note_catalog_body_search match 'folderrare'",[],|row|row.get::<_,i64>(0)).unwrap(),1);
    }
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_directory_delete_protects_root_bibliography_and_cascades_only_selected_relations()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(&store, &fixture.active, "main.tex", "root");
    seed(&store, &fixture.active, "notes/a.tex", "A");
    seed(&store, &fixture.active, "notes-old/keep.tex", "keep");
    seed(&store, &fixture.active, "managed/References.BIB", "managed");
    let protected = directory(&fixture, &store, delete(&fixture.active, "managed"))
        .await
        .unwrap_err();
    assert_eq!(protected.code, "managed_bibliography");
    fixture
        .database
        .lock()
        .unwrap()
        .execute(
            "update manuscripts set root_path='managed/root.tex' where id=?1",
            [&fixture.active],
        )
        .unwrap();
    let root = directory(&fixture, &store, delete(&fixture.active, "managed"))
        .await
        .unwrap_err();
    assert_eq!(root.code, "invalid_input");
    assert_eq!(root.message, "稿件路径无效");
    fixture
        .database
        .lock()
        .unwrap()
        .execute(
            "update manuscripts set root_path='main.tex' where id=?1",
            [&fixture.active],
        )
        .unwrap();
    session(
        &fixture,
        r#"["notes/a.tex","notes-old/keep.tex"]"#,
        "notes/a.tex",
        r#"{"notes/a.tex":1,"notes-old/keep.tex":2}"#,
    );
    draft(&fixture, "notes/a.tex");
    draft(&fixture, "notes-old/keep.tex");
    assert!(matches!(
        directory(&fixture, &store, delete(&fixture.active, "notes"))
            .await
            .unwrap(),
        FileResult::Deleted
    ));
    assert!(!source(&store, &fixture.active).join("notes").exists());
    assert!(
        source(&store, &fixture.active)
            .join("notes-old/keep.tex")
            .exists()
    );
    assert!(staged(&store, &fixture.active).is_empty());
    {
        let db = fixture.database.lock().unwrap();
        let row:(String,Option<String>,String)=db.query_row("select open_paths_json,active_path,cursor_state_json from writing_sessions where manuscript_id=?1",[&fixture.active],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.0).unwrap(),
            serde_json::json!(["notes-old/keep.tex"])
        );
        assert_eq!(row.1, None);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.2).unwrap(),
            serde_json::json!({"notes-old/keep.tex":2})
        );
        assert_eq!(db.query_row("select count(*) from manuscript_recovery_drafts where relative_path='notes-old/keep.tex'",[],|row|row.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(
            db.query_row(
                "select count(*) from manuscript_files where relative_path='notes/a.tex'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_directory_sql_failures_keep_original_order_and_restore_disk_after_confirmed_rollback()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(&store, &fixture.active, "chapters/a.tex", "A");
    session(&fixture, "null", "chapters/a.tex", "{}");
    fixture.database.lock().unwrap().execute_batch("create table synthetic_directory_prefix(value text); create temp trigger test_directory_fail before update on manuscript_files begin insert into synthetic_directory_prefix values('prefix'); select raise(fail,'PRIVATE_DIRECTORY_FAILURE');end;").unwrap();
    let error = directory(
        &fixture,
        &store,
        rename(&fixture.active, "chapters", "renamed"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::RolledBack));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.file-rename"
    );
    assert!(!error.message.contains("PRIVATE_DIRECTORY_FAILURE"));
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(
        source(&store, &fixture.active)
            .join("chapters/a.tex")
            .exists()
    );
    assert!(!source(&store, &fixture.active).join("renamed").exists());
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from synthetic_directory_prefix",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger test_directory_fail")
            .unwrap();
    }
    let error = directory(
        &fixture,
        &store,
        rename(&fixture.active, "chapters", "renamed"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.save_outcome, Some(SaveOutcome::RolledBack));
    assert!(error.message.contains("open paths JSON"));
    assert!(
        source(&store, &fixture.active)
            .join("chapters/a.tex")
            .exists()
    );
    // A failed SQL read is replayed only after the preceding index step.
    {
        let db = fixture.database.lock().unwrap();
        db.execute_batch("create temp trigger test_directory_delete_fail before delete on manuscript_files begin select raise(fail,'PRIVATE_DELETE_FAILURE');end;").unwrap();
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "writing_sessions",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    let error = directory(&fixture, &store, delete(&fixture.active, "chapters"))
        .await
        .unwrap_err();
    assert_eq!(error.save_outcome, Some(SaveOutcome::RolledBack));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.file-delete"
    );
    fixture
        .database
        .lock()
        .unwrap()
        .execute_batch("drop trigger test_directory_delete_fail")
        .unwrap();
    let error = directory(&fixture, &store, delete(&fixture.active, "chapters"))
        .await
        .unwrap_err();
    assert_eq!(error.save_outcome, Some(SaveOutcome::RolledBack));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.session"
    );
    assert!(
        source(&store, &fixture.active)
            .join("chapters/a.tex")
            .exists()
    );
    assert!(staged(&store, &fixture.active).is_empty());
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_directory_unconfirmed_rollback_fences_database_and_retains_staged_bytes() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    seed(&store, &fixture.active, "chapters/a.tex", "must survive");
    fixture.database.lock().unwrap().execute_batch("create temp trigger test_directory_rollback before delete on manuscript_files begin select raise(rollback,'PRIVATE_ROLLBACK');end;").unwrap();
    let error = directory(&fixture, &store, delete(&fixture.active, "chapters"))
        .await
        .unwrap_err();
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert!(error.save_rollback_error.is_some());
    assert!(
        error
            .message
            .contains("writing_directory_recovery_required")
    );
    assert!(!error.message.contains("PRIVATE_ROLLBACK"));
    assert!(store.connection().is_err());
    let retained = staged(&store, &fixture.active);
    assert_eq!(retained.len(), 1);
    assert_eq!(
        std::fs::read_to_string(retained[0].join("a.tex")).unwrap(),
        "must survive"
    );
    let reopened =
        rusqlite::Connection::open(fixture.directory.path().join("synthetic.sqlite3")).unwrap();
    assert_eq!(
        reopened
            .query_row(
                "select count(*) from manuscript_files where relative_path='chapters/a.tex'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    fixture.service.shutdown().await.unwrap();
}
