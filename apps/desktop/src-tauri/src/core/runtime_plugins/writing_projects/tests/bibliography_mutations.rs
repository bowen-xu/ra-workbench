//! Real management consumers against the actual independent W/L packages.
use super::bibliography_runtime::list;
use super::files::{seed, store};
use super::*;
use crate::runtime_plugins::{
    literature_trace::binding::NoteTraceBinding,
    writing_projects::files::{FileRequest, FileResult, bibliography::Call, operate_files},
};
use research_workbench_native_sdk::writing_bibliography::BibliographyEntryInput;
use rusqlite::params;
use serde_json::json;
pub(super) async fn call(f: &Fixture, store: &AppStore, call: Call) -> NativeResult<FileResult> {
    operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Bibliography { call },
    )
    .await
}
#[tokio::test]
#[ignore = "requires actual independent writing package; run the explicit C05 package acceptance"]
async fn actual_package_edits_original_entry_rejects_stale_cas_and_removes_only_unused_imports() {
    let f = Fixture::new();
    let store = store(&f);
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\cite{alpha}\\bibliography{refs}",
    );
    let original =
        "@article{alpha,title={Alpha},year={2024}}\n@article{beta,title={Unused},year={2020}}";
    seed(&store, &f.active, "refs.bib", original);
    f.database.lock().unwrap().execute("update manuscript_files set file_kind='bib' where manuscript_id=?1 and relative_path='refs.bib'",[&f.active]).unwrap();
    let before = list(&f, &store).await.unwrap();
    let alpha = before
        .iter()
        .find(|entry| entry.citation_key == "alpha")
        .unwrap();
    let mut draft = alpha.current.clone();
    draft["title"] = json!("Edited alpha");
    assert!(matches!(
        call(
            &f,
            &store,
            Call::Update {
                entry_id: alpha.id.clone(),
                expected: alpha.bibtex_entry.clone(),
                draft: draft.clone()
            }
        )
        .await
        .unwrap(),
        FileResult::BibliographyChanged
    ));
    let after = list(&f, &store).await.unwrap();
    let edited = after.iter().find(|entry| entry.id == alpha.id).unwrap();
    assert_eq!(edited.current["title"], "Edited alpha");
    assert_eq!(edited.imported, alpha.imported);
    assert_eq!(edited.original_bibtex, alpha.original_bibtex);
    let error = call(
        &f,
        &store,
        Call::Update {
            entry_id: alpha.id.clone(),
            expected: alpha.bibtex_entry.clone(),
            draft,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "file_conflict");
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert!(matches!(
        call(&f, &store, Call::RemoveUnused).await.unwrap(),
        FileResult::RemovedBibliographyEntries(1)
    ));
    let remaining = list(&f, &store).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, alpha.id);
    let bytes:String=f.database.lock().unwrap().query_row("select original_content from manuscript_bibliography_files where manuscript_id=?1 and relative_path='refs.bib'",[&f.active],|row|row.get(0)).unwrap();
    assert_eq!(bytes, original);
    f.service.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "requires actual independent writing and literature packages; run the explicit C05 package acceptance"]
async fn actual_packages_search_bind_link_and_rekey_keep_current_library_and_writing_identity() {
    let mut f = Fixture::new();
    let (_packages, literature, key) = super::bibliography_library::bind(&mut f);
    let store = store(&f).with_native_note_trace(NoteTraceBinding {
        service: Arc::clone(&literature),
        package_key: key,
    });
    {
        let db = f.database.lock().unwrap();
        for (id, title, updated) in [("a", "Live A", "2026-01-02"), ("b", "Live B", "2026-01-01")] {
            db.execute("insert into bibliographic_items(id,item_type,title,issued_year,bibtex_key,created_at,updated_at) values(?1,'journalArticle',?2,2024,'ImportedKey','original',?3)",params![id,title,updated]).unwrap();
        }
        db.execute(
            "insert into project_items(project_id,item_id,added_at) values('synthetic','b','original')",
            [],
        )
        .unwrap();
    }
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\cite{alpha,key}\\bibliography{refs}",
    );
    seed(
        &store,
        &f.active,
        "refs.bib",
        "@article{alpha,title={Imported},year={2020}}",
    );
    f.database.lock().unwrap().execute("update manuscript_files set file_kind='bib' where manuscript_id=?1 and relative_path='refs.bib'",[&f.active]).unwrap();
    let imported = list(&f, &store).await.unwrap().remove(0);
    let transactions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&transactions);
    f.database.lock().unwrap().authorizer(Some(
        move |context: rusqlite::hooks::AuthContext<'_>| {
            if let rusqlite::hooks::AuthAction::Transaction { operation } = context.action {
                observed.lock().unwrap().push(format!("{operation:?}"));
            }
            rusqlite::hooks::Authorization::Allow
        },
    ));
    let FileResult::CitationCandidates(candidates) = call(
        &f,
        &store,
        Call::Search {
            query: "".into(),
            scope: "library".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("candidates");
    };
    assert_eq!(
        candidates
            .iter()
            .map(|row| row.literature_item_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(candidates[0].citation_key, None);
    assert_eq!(candidates[0].warnings, ["缺少作者"]);
    let FileResult::CitationCandidates(project) = call(
        &f,
        &store,
        Call::Search {
            query: "".into(),
            scope: "project".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("project candidates");
    };
    assert_eq!(project.len(), 1);
    assert_eq!(project[0].literature_item_id, "b");
    f.database
        .lock()
        .unwrap()
        .authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>);
    {
        let events = transactions.lock().unwrap();
        assert!(!events.is_empty());
        assert!(
            events.chunks(2).all(|pair| pair == ["Begin", "Rollback"]),
            "{events:?}"
        );
    }
    let FileResult::CitationBinding(binding) = call(
        &f,
        &store,
        Call::Bind {
            entry: BibliographyEntryInput {
                literature_item_id: "a".into(),
                requested_key: Some("KEY".into()),
                bibtex_entry: "@article{original,title={Client title}}".into(),
            },
        },
    )
    .await
    .unwrap() else {
        panic!("binding");
    };
    assert_eq!(binding.citation_key, "key");
    assert!(matches!(
        call(
            &f,
            &store,
            Call::Link {
                entry_id: imported.id.clone(),
                item_id: "b".into()
            }
        )
        .await
        .unwrap(),
        FileResult::BibliographyChanged
    ));
    let rows = list(&f, &store).await.unwrap();
    let linked = rows.iter().find(|entry| entry.id == imported.id).unwrap();
    assert_eq!(linked.library_item_id.as_deref(), Some("b"));
    assert_eq!(linked.current["title"], "Live B");
    let error = call(
        &f,
        &store,
        Call::Update {
            entry_id: linked.id.clone(),
            expected: "stale".into(),
            draft: linked.current.clone(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "managed_bibliography");
    let assignments = rows
        .iter()
        .map(
            |row| research_workbench_native_sdk::writing_bibliography::rekey::Assignment {
                reference_id: row.id.clone(),
                base_key: "renamed".into(),
                from: row.citation_key.clone(),
                to: format!("new:{}", row.citation_key),
            },
        )
        .collect();
    assert!(matches!(
        call(&f, &store, Call::Rekey { assignments }).await.unwrap(),
        FileResult::CitationKeyMigration(_)
    ));
    let FileResult::CitationCandidates(found) = call(
        &f,
        &store,
        Call::Search {
            query: "new:key".into(),
            scope: "library".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("bound candidates");
    };
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].literature_item_id, "a");
    assert_eq!(found[0].citation_key.as_deref(), Some("new:key"));
    f.service.shutdown().await.unwrap();
    literature.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package; run the explicit C05 package acceptance"]
async fn actual_package_search_keeps_scope_error_before_absent_library_without_recovering_files() {
    let f = Fixture::new();
    let store = store(&f);
    f.database.lock().unwrap().execute("insert into manuscript_citation_key_writes(manuscript_id,payload_json) values(?1,'invalid pending payload')",[&f.active]).unwrap();
    let invalid = call(
        &f,
        &store,
        Call::Search {
            query: "".into(),
            scope: "invalid".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.code, "invalid_input");
    let unavailable = call(
        &f,
        &store,
        Call::Search {
            query: "".into(),
            scope: "library".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(unavailable.code, "literature_citations_unavailable");
    let pending: String = f
        .database
        .lock()
        .unwrap()
        .query_row(
            "select payload_json from manuscript_citation_key_writes where manuscript_id=?1",
            [&f.active],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, "invalid pending payload");
    f.service.shutdown().await.unwrap();
}
