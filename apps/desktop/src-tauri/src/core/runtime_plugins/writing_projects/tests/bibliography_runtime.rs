use super::files::{seed, store};
use super::*;
use crate::runtime_plugins::writing_projects::files::{
    FileRequest, FileResult, bibliography::Call, operate_files,
};

pub(super) async fn list(
    f: &Fixture,
    store: &AppStore,
) -> NativeResult<Vec<research_workbench_native_sdk::writing_bibliography::ProjectBibliographyEntry>>
{
    match operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Bibliography {
            call: Call::List {
                include_unused_automatic: false,
            },
        },
    )
    .await?
    {
        FileResult::Bibliography(entries) => Ok(entries),
        _ => panic!("full list"),
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_full_bibliography_list_adopts_publishes_and_detects_external_edits() {
    let f = Fixture::new();
    let store = store(&f);
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\documentclass{article}\n\\begin{document}\\cite{alpha}\\bibliography{refs}\\end{document}",
    );
    let source = "@article{alpha,title={Original title},year={2024}}";
    seed(&store, &f.active, "refs.bib", source);
    f.database.lock().unwrap().execute("update manuscript_files set file_kind='bib' where manuscript_id=?1 and relative_path='refs.bib'", [&f.active]).unwrap();
    let entries = list(&f, &store).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].citation_key, "alpha");
    assert_eq!(entries[0].current["title"], "Original title");
    assert_eq!(entries[0].paths, ["refs.bib"]);
    let (published, hash) = store.manuscript_files(&f.active).read("refs.bib").unwrap();
    assert!(published.starts_with("% Managed by Research Assistant."));
    assert_eq!(list(&f, &store).await.unwrap(), entries);
    let retained: String = f.database.lock().unwrap().query_row("select original_content from manuscript_bibliography_files where manuscript_id=?1 and relative_path='refs.bib'", [&f.active], |row| row.get(0)).unwrap();
    assert_eq!(retained, source);
    store
        .manuscript_files(&f.active)
        .save("refs.bib", "% external bytes\n", Some(&hash))
        .unwrap();
    let error = list(&f, &store).await.unwrap_err();
    assert_eq!(error.code, "bibliography_conflict");
    assert!(error.instance_id.is_some() && error.request_id.is_some());
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("refs.bib")
            .unwrap()
            .0,
        "% external bytes\n"
    );
    let FileResult::BibliographyStatus(status) = operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Bibliography {
            call: Call::Restore,
        },
    )
    .await
    .unwrap() else {
        panic!("restore status");
    };
    assert_eq!(status.status, "ready");
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("refs.bib")
            .unwrap()
            .0,
        published
    );
    let backup: String = f.database.lock().unwrap().query_row(
        "select relative_path from manuscript_files where manuscript_id=?1 and file_kind='other' and relative_path like 'refs.bib.backup-%'",
        [&f.active], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        store.manuscript_files(&f.active).read(&backup).unwrap().0,
        "% external bytes\n"
    );
    assert_eq!(list(&f, &store).await.unwrap(), entries);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_full_bibliography_restore_preserves_legacy_user_file_and_regenerates_target()
 {
    let f = Fixture::new();
    let store = store(&f);
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\documentclass{article}\n\\begin{document}\\cite{alpha}\\bibliography{references}\\end{document}",
    );
    let source = "% original user bytes\r\n@article{alpha,title={Preserved title}}\r\n";
    seed(&store, &f.active, "references.bib", source);
    f.database.lock().unwrap().execute("update manuscript_files set file_kind='bib' where manuscript_id=?1 and relative_path='references.bib'", [&f.active]).unwrap();
    let FileResult::BibliographyStatus(status) = operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Bibliography {
            call: Call::Restore,
        },
    )
    .await
    .unwrap() else {
        panic!("restore");
    };
    assert_eq!(status.status, "ready");
    let original: String = f.database.lock().unwrap().query_row(
        "select original_content from manuscript_bibliography_files where manuscript_id=?1 and relative_path='local.bib'", [&f.active], |row| row.get(0),
    ).unwrap();
    assert_eq!(original, source);
    let entries = list(&f, &store).await.unwrap();
    assert_eq!(entries[0].current["title"], "Preserved title");
    assert_eq!(entries[0].paths, ["local.bib"]);
    assert!(
        store
            .manuscript_files(&f.active)
            .read("references.bib")
            .unwrap()
            .0
            .contains("Preserved title")
    );
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_full_bibliography_list_unblocks_original_new_file_and_save_flow() {
    let f = Fixture::new();
    let store = store(&f);
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\documentclass{article}\n\\begin{document}Original\\end{document}",
    );
    assert!(list(&f, &store).await.unwrap().is_empty());
    let FileResult::BibliographyStatus(status) = operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Bibliography {
            call: Call::Sync { entries: vec![] },
        },
    )
    .await
    .unwrap() else {
        panic!("sync before new file");
    };
    assert_eq!(status.status, "ready");
    let request = super::file_save::request(&f.active, "intro.tex", "First file", None);
    let FileResult::Read(created) = operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Create { request },
    )
    .await
    .unwrap() else {
        panic!("create");
    };
    assert!(list(&f, &store).await.unwrap().is_empty());
    let request = super::file_save::request(
        &f.active,
        "intro.tex",
        "Saved second file",
        Some(&created.sha256),
    );
    let FileResult::Read(saved) = operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Save { request },
    )
    .await
    .unwrap() else {
        panic!("save");
    };
    assert_eq!(saved.content, "Saved second file");
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("intro.tex")
            .unwrap()
            .0,
        saved.content
    );
    f.service.shutdown().await.unwrap();
}
