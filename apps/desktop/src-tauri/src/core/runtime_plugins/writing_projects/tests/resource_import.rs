//! Original upload chain against the real independently built writing host.
use super::{
    bibliography_mutations::call,
    bibliography_runtime::list,
    files::{seed, store},
    *,
};
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{FileResult, bibliography::Call},
};
use std::{fs, path::PathBuf};

fn input(f: &Fixture, path: &str, bytes: &[u8]) -> String {
    let destination = f.directory.path().join("input").join(path);
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, bytes).unwrap();
    destination.to_str().unwrap().into()
}
async fn upload(f: &Fixture, store: &AppStore, paths: Vec<String>) -> NativeResult<FileResult> {
    call(
        f,
        store,
        Call::ImportResources {
            destination_directory: "assets".into(),
            source_paths: paths,
        },
    )
    .await
}
fn root(f: &Fixture, store: &AppStore) -> PathBuf {
    crate::scoped_text_files::legacy::manuscript_source_directory(
        store.manuscript_directory(),
        &f.active,
    )
    .unwrap()
}
fn assert_staging_released(f: &Fixture, store: &AppStore) {
    assert!(
        fs::read_dir(root(f, store).parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".writing-resources-"))
    );
}

#[tokio::test]
#[ignore = "requires the independently built writing v2 package"]
async fn actual_package_uploads_binary_without_adopting_unrelated_legacy_bibliography() {
    let f = Fixture::new();
    let store = store(&f);
    let main = "\\documentclass{article}\n\\begin{document}Original\\end{document}";
    seed(&store, &f.active, "main.tex", main);
    let legacy = "@book{2026,title={Legacy}}\n";
    seed(&store, &f.active, "legacy.bib", legacy);
    let bytes = b"\x89PNG\0synthetic\xff";
    let source = input(&f, "figure.png", bytes);
    let FileResult::ResourcesImported(result) =
        upload(&f, &store, vec![source.clone()]).await.unwrap()
    else {
        panic!("upload result")
    };
    assert_eq!(result.imported_paths, ["assets/figure.png"]);
    assert_eq!(result.byte_size, bytes.len() as u64);
    assert_eq!(
        fs::read(root(&f, &store).join("assets/figure.png")).unwrap(),
        bytes
    );
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("main.tex")
            .unwrap()
            .0,
        main
    );
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("legacy.bib")
            .unwrap()
            .0,
        legacy
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_bibliography_entries",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        upload(&f, &store, vec![source]).await.unwrap_err().code,
        "file_conflict"
    );
    assert_staging_released(&f, &store);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the independently built writing v2 package"]
async fn actual_package_upload_normalizes_bib_and_new_tex_preserves_originals_and_cold_reopens() {
    let f = Fixture::new();
    let store = store(&f);
    let main = "\\documentclass{article}\n\\begin{document}Original\\bibliography{assets/refs}\\end{document}";
    seed(&store, &f.active, "main.tex", main);
    let original = "% original upload\r\n@string{journal={Synthetic}}\r\n@article{2026,title={Imported},journal=journal,custom={Keep}}\r\n";
    let bib = input(&f, "refs.bib", original.as_bytes());
    let chapter = input(&f, "chapter.tex", b"\\section{New}\\cite{2026}\n");
    let FileResult::ResourcesImported(result) =
        upload(&f, &store, vec![bib, chapter]).await.unwrap()
    else {
        panic!("upload result")
    };
    assert_eq!(
        result.imported_paths,
        ["assets/refs.bib", "assets/chapter.tex"]
    );
    let entries = list(&f, &store).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_ne!(entries[0].citation_key, "2026");
    assert_eq!(entries[0].current["title"], "Imported");
    assert_eq!(entries[0].original_key, "2026");
    let (chapter, chapter_hash) = store
        .manuscript_files(&f.active)
        .read("assets/chapter.tex")
        .unwrap();
    assert!(chapter.contains(&format!("\\cite{{{}}}", entries[0].citation_key)));
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("main.tex")
            .unwrap()
            .0,
        main
    );
    let retained:String=store.connection().unwrap().query_row("select original_content from manuscript_bibliography_files where manuscript_id=?1 and relative_path='assets/refs.bib'",[&f.active],|r|r.get(0)).unwrap();
    assert_eq!(retained, original);
    let published = store
        .manuscript_files(&f.active)
        .read("assets/refs.bib")
        .unwrap();
    assert!(published.0.contains("Managed by Research Assistant"));
    assert!(published.0.contains("custom={Keep}"));
    assert_staging_released(&f, &store);
    let Fixture {
        directory,
        database,
        service,
        catalog,
        key,
        active,
        ..
    } = f;
    service.shutdown().await.unwrap();
    drop(service);
    drop(store);
    drop(database);
    let mut sql = rusqlite::Connection::open(directory.path().join("synthetic.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let database = Database::open(sql, &directory.path().join("data-log")).unwrap();
    let store = AppStore::for_test_database(database, directory.path().join("objects"));
    let service = Arc::new(NativePreviewService::new(Arc::clone(&catalog)));
    let FileResult::Bibliography(reopened) =
        crate::runtime_plugins::writing_projects::files::operate_files(
            store.clone(),
            Arc::clone(&service),
            catalog,
            key,
            active.clone(),
            crate::runtime_plugins::writing_projects::files::FileRequest::Bibliography {
                call: Call::List {
                    include_unused_automatic: false,
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("reopen")
    };
    assert_eq!(reopened, entries);
    assert_eq!(
        store
            .manuscript_files(&active)
            .read("assets/refs.bib")
            .unwrap(),
        published
    );
    assert_eq!(
        store
            .manuscript_files(&active)
            .read("assets/chapter.tex")
            .unwrap(),
        (chapter, chapter_hash)
    );
    assert_eq!(
        store.manuscript_files(&active).read("main.tex").unwrap().0,
        main
    );
    service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the independently built writing v2 package"]
async fn actual_package_upload_rejects_invalid_conflicting_and_existing_document_inputs() {
    let f = Fixture::new();
    let store = store(&f);
    let main = "\\documentclass{article}\n\\begin{document}\\cite{2026}\\end{document}";
    seed(&store, &f.active, "main.tex", main);
    let one = input(&f, "one/shared.png", b"one");
    let two = input(&f, "two/shared.png", b"two");
    assert_eq!(
        upload(&f, &store, vec![]).await.unwrap_err().code,
        "file_too_large"
    );
    assert_eq!(
        upload(&f, &store, vec![one.clone(); 257])
            .await
            .unwrap_err()
            .code,
        "file_too_large"
    );
    assert_eq!(
        call(
            &f,
            &store,
            Call::ImportResources {
                destination_directory: "../escape".into(),
                source_paths: vec![one.clone()]
            }
        )
        .await
        .unwrap_err()
        .code,
        "invalid_input"
    );
    assert_eq!(
        upload(&f, &store, vec!["relative.png".into()])
            .await
            .unwrap_err()
            .code,
        "invalid_input"
    );
    assert_eq!(
        upload(&f, &store, vec![one, two]).await.unwrap_err().code,
        "file_conflict"
    );
    let malformed = input(&f, "broken.bib", b"@book{unfinished");
    assert_eq!(
        upload(&f, &store, vec![malformed]).await.unwrap_err().code,
        "bibliography_import"
    );
    let bib = input(&f, "refs.bib", b"@book{2026,title={New}}\n");
    let rejected = upload(&f, &store, vec![bib]).await.unwrap_err();
    assert_eq!(rejected.code, "bibliography_import");
    assert!(rejected.instance_id.is_some() && rejected.request_id.is_some());
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("main.tex")
            .unwrap()
            .0,
        main
    );
    assert!(!root(&f, &store).join("assets/refs.bib").exists());
    assert!(!root(&f, &store).join("assets/shared.png").exists());
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from manuscript_bibliography_entries",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_staging_released(&f, &store);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the independently built writing v2 package"]
async fn actual_package_upload_rolls_back_file_registration_and_retracts_published_bytes() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", "Original");
    let source = input(&f, "figure.png", b"synthetic binary");
    store.connection().unwrap().execute_batch("create trigger fail_resource_registration before insert on manuscript_files when new.relative_path='assets/figure.png' begin select raise(abort,'synthetic resource registration'); end;").unwrap();
    let error = upload(&f, &store, vec![source.clone()]).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.save_outcome, Some(SaveOutcome::RolledBack));
    assert!(!root(&f, &store).join("assets/figure.png").exists());
    assert!(store.connection().unwrap().is_autocommit());
    assert_staging_released(&f, &store);
    store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_resource_registration")
        .unwrap();
    assert!(matches!(
        upload(&f, &store, vec![source]).await.unwrap(),
        FileResult::ResourcesImported(_)
    ));
    f.service.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires the independently built writing v2 package"]
async fn actual_package_upload_rejects_symlink_before_publication() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", "Original");
    let source = input(&f, "figure.png", b"synthetic");
    let link = f.directory.path().join("input/link.png");
    std::os::unix::fs::symlink(&source, &link).unwrap();
    assert_eq!(
        upload(&f, &store, vec![link.to_str().unwrap().into()])
            .await
            .unwrap_err()
            .code,
        "unsafe_path"
    );
    assert!(!root(&f, &store).join("assets/link.png").exists());
    assert_staging_released(&f, &store);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the independently built writing v2 package"]
async fn actual_package_upload_recovers_committed_bibliography_after_projection_failure() {
    let f = Fixture::new();
    let store = store(&f);
    let main = "\\documentclass{article}\n\\begin{document}Original\\bibliography{assets/refs}\\end{document}";
    seed(&store, &f.active, "main.tex", main);
    let original = "% source bytes\r\n@book{alpha,title={Imported},custom={Keep}}\r\n";
    let bib = input(&f, "refs.bib", original.as_bytes());
    store.connection().unwrap().execute_batch("create trigger fail_resource_projection before update of content_hash on manuscript_bibliography_files when new.relative_path='assets/refs.bib' begin select raise(abort,'synthetic projection failure'); end;").unwrap();
    let error = upload(&f, &store, vec![bib.clone()]).await.unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert!(matches!(error.save_outcome, Some(SaveOutcome::Unknown)));
    let original_row: (String, String) = store
        .connection()
        .unwrap()
        .query_row(
            "select id,original_bibtex from manuscript_bibliography_entries where manuscript_id=?1",
            [&f.active],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(original_row.1.contains("custom={Keep}"));
    let retained: String = store.connection().unwrap().query_row("select original_content from manuscript_bibliography_files where manuscript_id=?1 and relative_path='assets/refs.bib'", [&f.active], |row| row.get(0)).unwrap();
    assert_eq!(retained, original);
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("main.tex")
            .unwrap()
            .0,
        main
    );
    assert_staging_released(&f, &store);
    store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_resource_projection")
        .unwrap();
    let recovered = list(&f, &store).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, original_row.0);
    assert_eq!(recovered[0].current["title"], "Imported");
    assert_eq!(
        upload(&f, &store, vec![bib]).await.unwrap_err().code,
        "file_conflict"
    );
    assert_eq!(list(&f, &store).await.unwrap(), recovered);
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("main.tex")
            .unwrap()
            .0,
        main
    );
    f.service.shutdown().await.unwrap();
}
