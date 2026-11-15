use super::*;
use crate::{
    runtime_plugins::{
        literature_catalog::save::SaveOutcome,
        writing_projects::files::{FileRequest, FileResult, operate_files},
    },
    scoped_text_files::legacy::sha256,
};
use research_workbench_native_sdk::writing_project::CitationKeyWrite;

pub(in crate::runtime_plugins::writing_projects) fn store(fixture: &Fixture) -> AppStore {
    AppStore::for_test_database(
        Arc::clone(&fixture.database),
        fixture.directory.path().join("objects"),
    )
}
pub(in crate::runtime_plugins::writing_projects) fn seed(
    store: &AppStore,
    id: &str,
    path: &str,
    content: &str,
) -> String {
    let hash = store
        .manuscript_files(id)
        .save(path, content, None)
        .unwrap();
    store.connection().unwrap().execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,?2,'tex',?3,?4,'original')", rusqlite::params![id,path,content.len() as i64,hash]).unwrap();
    hash
}
pub(super) fn journal(fixture: &Fixture, writes: &[CitationKeyWrite]) {
    fixture.database.lock().unwrap().execute("insert into manuscript_citation_key_writes values(?1,?2) on conflict(manuscript_id) do update set payload_json=excluded.payload_json", rusqlite::params![fixture.active,serde_json::to_string(writes).unwrap()]).unwrap();
}
async fn read(
    fixture: &Fixture,
    store: &AppStore,
    id: &str,
    request: FileRequest,
) -> NativeResult<FileResult> {
    operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        id.into(),
        request,
    )
    .await
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_lists_and_reads_original_files_without_requiring_an_indexed_read() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let hash = seed(&store, &fixture.active, "sections/a.tex", "正文\r\n");
    seed(&store, &fixture.active, "B.tex", "B");
    store
        .manuscript_files(&fixture.active)
        .save("unindexed.tex", "unindexed", None)
        .unwrap();
    let FileResult::List(items) = read(
        &fixture,
        &store,
        &format!(" {} ", fixture.active),
        FileRequest::List,
    )
    .await
    .unwrap() else {
        panic!("list");
    };
    assert_eq!(
        items
            .iter()
            .map(|item| item.relative_path.as_str())
            .collect::<Vec<_>>(),
        ["B.tex", "sections/a.tex"]
    );
    assert_eq!(items[1].sha256, hash);
    let FileResult::Read(file) = read(
        &fixture,
        &store,
        &fixture.active,
        FileRequest::Read {
            relative_path: " sections/a.tex ".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("read");
    };
    assert_eq!(file.relative_path, " sections/a.tex ");
    assert_eq!(file.content, "正文\r\n");
    assert_eq!(file.sha256, hash);
    let FileResult::Read(file) = read(
        &fixture,
        &store,
        &fixture.active,
        FileRequest::Read {
            relative_path: "unindexed.tex".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("read unindexed");
    };
    assert_eq!(file.content, "unindexed");
    let missing = read(
        &fixture,
        &store,
        "missing",
        FileRequest::Read {
            relative_path: "../bad.tex".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(missing.code, "not_found");
    assert_eq!(missing.message, "稿件不存在");
    assert_eq!(missing.save_outcome, Some(SaveOutcome::NotStarted));
    let invalid = read(
        &fixture,
        &store,
        &fixture.active,
        FileRequest::Read {
            relative_path: "../bad.tex".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.code, "invalid_input");
    assert!(invalid.instance_id.is_some() && invalid.request_id.is_some());
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_file_reads_recover_pending_citation_bytes_and_preserve_external_conflicts()
{
    let fixture = Fixture::new();
    let store = store(&fixture);
    let old = seed(&store, &fixture.active, "main.tex", "\\cite{old}");
    let write = CitationKeyWrite {
        path: "main.tex".into(),
        previous: old,
        content: "\\cite{new}\r\n".into(),
    };
    journal(&fixture, std::slice::from_ref(&write));
    let FileResult::Read(file) = read(
        &fixture,
        &store,
        &fixture.active,
        FileRequest::Read {
            relative_path: "main.tex".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("read");
    };
    assert_eq!(file.content, write.content);
    assert_eq!(file.sha256, sha256(write.content.as_bytes()));
    {
        let db = fixture.database.lock().unwrap();
        let indexed: (String, String, i64) = db.query_row("select sha256,file_kind,byte_size from manuscript_files where manuscript_id=?1 and relative_path='main.tex'", [&fixture.active], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(
            indexed,
            (
                file.sha256.clone(),
                "tex".into(),
                write.content.len() as i64
            )
        );
        assert_eq!(
            db.query_row(
                "select count(*) from manuscript_citation_key_writes",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    // A lost clear/ACK can leave the same plan after the file already reached
    // its target. Recovering it still updates the original index and clears it.
    journal(&fixture, std::slice::from_ref(&write));
    read(&fixture, &store, &fixture.active, FileRequest::List)
        .await
        .unwrap();
    journal(
        &fixture,
        &[CitationKeyWrite {
            content: "next".into(),
            previous: file.sha256,
            ..write
        }],
    );
    store
        .manuscript_files(&fixture.active)
        .save("main.tex", "external", None)
        .unwrap();
    let error = read(&fixture, &store, &fixture.active, FileRequest::List)
        .await
        .unwrap_err();
    assert_eq!(error.code, "file_conflict");
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .unwrap()
            .0,
        "external"
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
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_file_recovery_keeps_saved_bytes_pending_plan_and_sql_failure_prefix() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let old = seed(&store, &fixture.active, "main.tex", "old");
    let write = CitationKeyWrite {
        path: "main.tex".into(),
        previous: old,
        content: "next".into(),
    };
    journal(&fixture, std::slice::from_ref(&write));
    fixture.database.lock().unwrap().execute_batch("create table synthetic_redo_prefix(value text);create temp trigger test_redo_index_fail before update on manuscript_files begin insert into synthetic_redo_prefix values('prefix'); select raise(fail,'PRIVATE_REDO_FAIL'); end;").unwrap();
    let error = read(&fixture, &store, &fixture.active, FileRequest::List)
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.file-upsert"
    );
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(!error.message.contains("PRIVATE_REDO_FAIL"));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .unwrap()
            .0,
        write.content
    );
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(
            db.query_row("select count(*) from synthetic_redo_prefix", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "select count(*) from manuscript_citation_key_writes",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        db.execute_batch("drop trigger test_redo_index_fail")
            .unwrap();
    }
    // Explicit retry follows the same journal and the original target-hash rule.
    read(&fixture, &store, &fixture.active, FileRequest::List)
        .await
        .unwrap();
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
        0
    );
    fixture.service.shutdown().await.unwrap();
}
