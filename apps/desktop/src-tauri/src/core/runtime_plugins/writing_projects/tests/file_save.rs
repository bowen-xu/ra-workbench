use super::files::{journal, seed, store};
use super::*;
use crate::{
    runtime_plugins::{
        literature_catalog::save::SaveOutcome,
        writing_projects::files::{FileRequest, FileResult, operate_files},
    },
    scoped_text_files::legacy::sha256,
};
use research_workbench_native_sdk::writing_project::{CitationKeyWrite, SaveFileRequest};

pub(super) fn request(
    id: &str,
    path: &str,
    content: &str,
    expected: Option<&str>,
) -> SaveFileRequest {
    SaveFileRequest {
        manuscript_id: id.into(),
        relative_path: path.into(),
        content: content.into(),
        expected_sha256: expected.map(str::to_owned),
    }
}
async fn save(
    fixture: &Fixture,
    store: &AppStore,
    request: SaveFileRequest,
) -> NativeResult<research_workbench_native_sdk::writing_project::ManuscriptFileContent> {
    match operate_files(
        store.clone(),
        Arc::clone(&fixture.service),
        Arc::clone(&fixture.catalog),
        fixture.key.clone(),
        request.manuscript_id.clone(),
        FileRequest::Save { request },
    )
    .await?
    {
        FileResult::Read(file) => Ok(file),
        _ => panic!("saved file"),
    }
}
pub(super) fn note(id: &str, text: &str) -> String {
    serde_json::json!({"schemaVersion":1,"noteId":id,"blocks":[{"id":"block","type":"paragraph","props":{},"content":{"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]},"children":[]}]}).to_string()
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_file_save_preserves_raw_output_cas_and_original_error_order() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let old = seed(&store, &fixture.active, "main.tex", "old");
    let saved = save(
        &fixture,
        &store,
        request(
            &format!(" {} ", fixture.active),
            " main.tex ",
            "正文\r\n",
            Some(&old),
        ),
    )
    .await
    .unwrap();
    assert_eq!(saved.relative_path, " main.tex ");
    assert_eq!(saved.content, "正文\r\n");
    assert_eq!(saved.sha256, sha256(saved.content.as_bytes()));
    let conflict = save(
        &fixture,
        &store,
        request(&fixture.active, "main.tex", "stale", Some(&old)),
    )
    .await
    .unwrap_err();
    assert_eq!(conflict.code, "file_conflict");
    assert_eq!(conflict.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .unwrap()
            .0,
        saved.content
    );
    let bib = save(
        &fixture,
        &store,
        request("missing", " Refs.BIB ", "x", None),
    )
    .await
    .unwrap_err();
    assert_eq!(bib.code, "managed_bibliography");
    let missing = save(
        &fixture,
        &store,
        request("missing", "../invalid.tex", "x", None),
    )
    .await
    .unwrap_err();
    assert_eq!(missing.code, "not_found");
    assert_eq!(missing.message, "稿件不存在");
    let invalid = save(
        &fixture,
        &store,
        request(&fixture.active, "../invalid.tex", "x", None),
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.code, "invalid_input");
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_note_save_updates_same_catalog_and_preserves_invalid_files_and_duplicate_identity()
 {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let good = note("stable", "searchrare研究");
    let first = save(
        &fixture,
        &store,
        request(&fixture.active, "notes/思路.RANOTE", &good, None),
    )
    .await
    .unwrap();
    {
        let db = fixture.database.lock().unwrap();
        let row: (String,String,String,String,String) = db.query_row("select note_id,title,state,search_text,folded_text from writing_note_catalog join writing_note_catalog_body using(manuscript_id,relative_path) where manuscript_id=?1 and relative_path='notes/思路.RANOTE'", [&fixture.active], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
        assert_eq!(
            row,
            (
                "stable".into(),
                "思路".into(),
                "ready".into(),
                "searchrare研究".into(),
                research_note_query::note_fold("searchrare研究")
            )
        );
    }
    save(
        &fixture,
        &store,
        request(&fixture.active, "second.ranote", &good, None),
    )
    .await
    .unwrap();
    {
        let db = fixture.database.lock().unwrap();
        let row: (String,String) = db.query_row("select state,error_cause from writing_note_catalog where manuscript_id=?1 and relative_path='second.ranote'", [&fixture.active], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(row.0, "error");
        assert!(row.1.starts_with("writing_note_identity_duplicate"));
    }
    let invalid = save(
        &fixture,
        &store,
        request(
            &fixture.active,
            "notes/思路.RANOTE",
            "not json",
            Some(&first.sha256),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("notes/思路.RANOTE")
            .unwrap()
            .0,
        "not json"
    );
    assert_eq!(invalid.sha256, sha256(b"not json"));
    {
        let db = fixture.database.lock().unwrap();
        let row: (Option<String>,String,String) = db.query_row("select note_id,state,error_cause from writing_note_catalog where manuscript_id=?1 and relative_path='notes/思路.RANOTE'", [&fixture.active], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(row.0, None);
        assert_eq!(row.1, "error");
        assert!(row.2.starts_with("writing_note_json_invalid"));
        assert_eq!(db.query_row("select count(*) from writing_note_catalog_body_search where writing_note_catalog_body_search match 'searchrare'", [], |row|row.get::<_,i64>(0)).unwrap(),0);
    }
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_note_index_failure_rolls_back_sql_but_keeps_saved_file_for_explicit_retry()
{
    let fixture = Fixture::new();
    let store = store(&fixture);
    let original = note("stable", "oldtext");
    let next = note("stable", "newtext");
    let first = save(
        &fixture,
        &store,
        request(&fixture.active, "note.ranote", &original, None),
    )
    .await
    .unwrap();
    fixture.database.lock().unwrap().execute_batch("create table synthetic_note_prefix(value text);create temp trigger test_note_body_fail before insert on writing_note_catalog_body begin insert into synthetic_note_prefix values('prefix');select raise(fail,'PRIVATE_NOTE_BODY');end;").unwrap();
    let error = save(
        &fixture,
        &store,
        request(&fixture.active, "note.ranote", &next, Some(&first.sha256)),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert_eq!(
        error.data_error.unwrap().resource_id,
        "writing.projects.note-body-upsert"
    );
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(!error.message.contains("PRIVATE_NOTE_BODY"));
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("note.ranote")
            .unwrap()
            .0,
        next
    );
    {
        let db = fixture.database.lock().unwrap();
        assert_eq!(db.query_row("select sha256 from manuscript_files where manuscript_id=?1 and relative_path='note.ranote'",[&fixture.active],|row|row.get::<_,String>(0)).unwrap(),first.sha256);
        assert_eq!(db.query_row("select search_text from writing_note_catalog_body where manuscript_id=?1 and relative_path='note.ranote'",[&fixture.active],|row|row.get::<_,String>(0)).unwrap(),"oldtext");
        assert_eq!(
            db.query_row("select count(*) from synthetic_note_prefix", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        db.execute_batch("drop trigger test_note_body_fail")
            .unwrap();
    }
    let expected = sha256(next.as_bytes());
    save(
        &fixture,
        &store,
        request(&fixture.active, "note.ranote", &next, Some(&expected)),
    )
    .await
    .unwrap();
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_save_recovers_pending_bibliography_before_rejecting_managed_file() {
    let fixture = Fixture::new();
    let store = store(&fixture);
    let previous = seed(&store, &fixture.active, "main.tex", "old");
    journal(
        &fixture,
        &[CitationKeyWrite {
            path: "main.tex".into(),
            previous,
            content: "recovered".into(),
        }],
    );
    let error = save(
        &fixture,
        &store,
        request(&fixture.active, "refs.bib", "forbidden", None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "managed_bibliography");
    assert_eq!(
        error.save_outcome,
        Some(SaveOutcome::Committed {
            item_id: fixture.active.clone()
        })
    );
    assert_eq!(
        store
            .manuscript_files(&fixture.active)
            .read("main.tex")
            .unwrap()
            .0,
        "recovered"
    );
    assert!(
        !store
            .manuscript_directory()
            .join(&fixture.active)
            .join("source/refs.bib")
            .exists()
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
        0
    );
    fixture.service.shutdown().await.unwrap();
}
