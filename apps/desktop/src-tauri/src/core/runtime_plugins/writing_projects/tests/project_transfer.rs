//! Complete original project flow against the actual independently built host.
use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{
        FileRequest, FileResult,
        bibliography::{Call, project_transfer::commands::run_import},
        operate_files,
    },
};
use research_workbench_native_sdk::writing_project::ManuscriptTransferResult;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const MAIN: &str = "\\documentclass{article}\n\\begin{document}\\input{sections/results}\\cite{2026}\\bibliographystyle{plain}\\bibliography{references}\\end{document}\n";
const BIB: &str = "% original source\r\n@article{2026,author={Preview, Alex},title={Synthetic transfer},year={2026},custom={Keep me}}\r\n";

fn input(f: &Fixture) -> PathBuf {
    let root = f.directory.path().join("input");
    fs::create_dir_all(root.join("sections")).unwrap();
    fs::create_dir_all(root.join("figures")).unwrap();
    fs::write(root.join("main.tex"), MAIN).unwrap();
    fs::write(root.join("sections/results.tex"), "Synthetic result.\r\n").unwrap();
    fs::write(root.join("references.bib"), BIB).unwrap();
    fs::write(root.join("figures/a.png"), b"\x89PNG\0synthetic\xff").unwrap();
    root
}

async fn import(
    f: &Fixture,
    store: &AppStore,
    source: &Path,
    requested: Option<&str>,
) -> NativeResult<ManuscriptTransferResult> {
    run_import(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        Call::ImportProject {
            title: " Imported transfer ".into(),
            source_path: source.to_string_lossy().into(),
            requested_root: requested.map(str::to_owned),
        },
    )
    .await
}

fn source(store: &AppStore, id: &str) -> PathBuf {
    crate::scoped_text_files::legacy::manuscript_source_directory(store.manuscript_directory(), id)
        .unwrap()
}

fn bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut pending = vec![root.to_owned()];
    let mut result = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_str().unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    result
}

async fn export(
    f: &Fixture,
    store: &AppStore,
    id: &str,
    destination: &Path,
    rename: bool,
) -> NativeResult<research_workbench_native_sdk::writing_project::ManuscriptExportResult> {
    match operate_files(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        id.into(),
        FileRequest::ExportSource {
            destination: destination.to_string_lossy().into(),
            rename_on_conflict: rename,
        },
    )
    .await?
    {
        FileResult::Exported(result) => Ok(result),
        _ => panic!("source export"),
    }
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_imports_duplicates_exports_and_cold_reopens_the_same_project() {
    let f = Fixture::new();
    let store = files::store(&f);
    let input = input(&f);
    let original = bytes(&input);
    let imported = import(&f, &store, &input, Some("main.tex")).await.unwrap();
    let id = imported.manuscript.id.clone();
    assert_eq!(imported.manuscript.title, "Imported transfer");
    assert_eq!(imported.file_count, 4);
    assert_eq!(imported.root_candidates, ["main.tex"]);
    assert_eq!(bytes(&input), original);
    let entries = crate::runtime_plugins::writing_projects::files::operate_files(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        id.clone(),
        FileRequest::Bibliography {
            call: Call::List {
                include_unused_automatic: false,
            },
        },
    )
    .await
    .unwrap();
    let FileResult::Bibliography(entries) = entries else {
        panic!("bibliography")
    };
    assert_eq!(entries.len(), 1);
    assert_ne!(entries[0].citation_key, "2026");
    assert_eq!(entries[0].original_key, "2026");
    let imported_bytes = bytes(&source(&store, &id));
    assert!(
        String::from_utf8(imported_bytes["main.tex"].clone())
            .unwrap()
            .contains(&format!("\\cite{{{}}}", entries[0].citation_key))
    );
    assert_eq!(
        imported_bytes["sections/results.tex"],
        original["sections/results.tex"]
    );
    assert_eq!(imported_bytes["figures/a.png"], original["figures/a.png"]);
    let imported_original: String = store.connection().unwrap().query_row("select original_content from manuscript_bibliography_files where manuscript_id=?1 and relative_path='references.bib'", [&id], |row| row.get(0)).unwrap();
    assert_eq!(imported_original, BIB);
    let duplicate = run_import(
        store.clone(),
        f.service.clone(),
        f.catalog.clone(),
        f.key.clone(),
        Call::DuplicateProject {
            source_id: id.clone(),
        },
    )
    .await
    .unwrap();
    assert_ne!(duplicate.manuscript.id, id);
    assert_eq!(duplicate.manuscript.title, "Imported transfer（副本）");
    assert_eq!(
        fs::read(source(&store, &duplicate.manuscript.id).join("main.tex")).unwrap(),
        imported_bytes["main.tex"]
    );
    assert_eq!(bytes(&source(&store, &id)), imported_bytes);
    fs::write(
        source(&store, &id).join(".agent-before-original.bak"),
        "preserved recovery",
    )
    .unwrap();
    let destination = f.directory.path().join("export.zip");
    let exported = export(&f, &store, &id, &destination, false).await.unwrap();
    assert_eq!(exported.file_count, 4);
    let mut archive = ZipArchive::new(fs::File::open(&destination).unwrap()).unwrap();
    assert_eq!(archive.len(), 4);
    for (path, expected) in &imported_bytes {
        let mut actual = Vec::new();
        archive
            .by_name(path)
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(&actual, expected);
    }
    assert_eq!(
        export(&f, &store, &id, &destination, false)
            .await
            .unwrap_err()
            .code,
        "file_conflict"
    );
    let numbered = export(&f, &store, &id, &destination, true).await.unwrap();
    assert_eq!(
        Path::new(&numbered.destination_path),
        f.directory
            .path()
            .canonicalize()
            .unwrap()
            .join("export (1).zip")
    );
    let roundtrip = import(&f, &store, &destination, None).await.unwrap();
    assert_ne!(roundtrip.manuscript.id, id);
    assert_eq!(
        fs::read(source(&store, &roundtrip.manuscript.id).join("main.tex")).unwrap(),
        imported_bytes["main.tex"]
    );
    let all_before = bytes(&source(&store, &id));
    let Fixture {
        directory,
        database,
        service,
        catalog,
        key,
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
    let service = Arc::new(NativePreviewService::new(catalog.clone()));
    let result = operate_files(
        store.clone(),
        service.clone(),
        catalog,
        key,
        id.clone(),
        FileRequest::Bibliography {
            call: Call::List {
                include_unused_automatic: false,
            },
        },
    )
    .await
    .unwrap();
    let FileResult::Bibliography(reopened) = result else {
        panic!("cold bibliography")
    };
    assert_eq!(reopened, entries);
    assert_eq!(bytes(&source(&store, &id)), all_before);
    service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_import_rejections_and_sql_failure_remove_only_the_new_project() {
    let f = Fixture::new();
    let store = files::store(&f);
    let input = input(&f);
    let original = bytes(&input);
    let baseline: i64 = store
        .connection()
        .unwrap()
        .query_row("select count(*) from manuscripts", [], |row| row.get(0))
        .unwrap();
    let rejected = import(&f, &store, &input, Some("missing.tex"))
        .await
        .unwrap_err();
    assert_eq!(rejected.code, "invalid_input");
    assert_eq!(rejected.save_outcome, Some(SaveOutcome::RolledBack));
    let rejected_id = rejected.operation_id.unwrap();
    assert!(!source(&store, &rejected_id).parent().unwrap().exists());
    store.connection().unwrap().execute_batch("create trigger fail_transfer_file before insert on manuscript_files begin select raise(abort,'synthetic import fault'); end;").unwrap();
    let failed = import(&f, &store, &input, None).await.unwrap_err();
    assert_eq!(failed.code, "storage_error");
    assert_eq!(failed.save_outcome, Some(SaveOutcome::RolledBack));
    let failed_id = failed.operation_id.unwrap();
    assert!(!source(&store, &failed_id).parent().unwrap().exists());
    let db = store.connection().unwrap();
    assert_eq!(
        db.query_row("select count(*) from manuscripts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        baseline
    );
    assert_eq!(
        db.query_row("select count(*) from manuscript_files", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("drop trigger fail_transfer_file;")
        .unwrap();
    drop(db);
    assert_eq!(bytes(&input), original);
    import(&f, &store, &input, None).await.unwrap();
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_zip_import_uses_original_root_prefix_and_rejects_zip_slip() {
    let f = Fixture::new();
    let store = files::store(&f);
    let input = f.directory.path().join("source.zip");
    let mut zip = ZipWriter::new(fs::File::create(&input).unwrap());
    for (path, text) in [
        ("paper/main.tex", "\\documentclass{article} main"),
        (
            "paper/appendix/root.tex",
            "\\documentclass{article} appendix",
        ),
        ("__MACOSX/ignored", "ignored"),
    ] {
        zip.start_file(path, SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let result = import(&f, &store, &input, Some(" appendix/root.tex "))
        .await
        .unwrap();
    assert_eq!(result.file_count, 2);
    assert_eq!(result.manuscript.root_path, "appendix/root.tex");
    assert_eq!(result.root_candidates, ["main.tex", "appendix/root.tex"]);
    assert!(!source(&store, &result.manuscript.id).join("paper").exists());
    let unsafe_zip = f.directory.path().join("unsafe.zip");
    let mut zip = ZipWriter::new(fs::File::create(&unsafe_zip).unwrap());
    zip.start_file("../escape.tex", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(MAIN.as_bytes()).unwrap();
    zip.finish().unwrap();
    let failure = import(&f, &store, &unsafe_zip, None).await.unwrap_err();
    assert_eq!(failure.code, "invalid_input");
    assert_eq!(failure.save_outcome, Some(SaveOutcome::RolledBack));
    assert!(!f.directory.path().join("escape.tex").exists());
    assert!(
        !source(&store, &failure.operation_id.unwrap())
            .parent()
            .unwrap()
            .exists()
    );
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_exports_keep_original_destination_and_missing_pdf_errors() {
    let f = Fixture::new();
    let store = files::store(&f);
    let binding = super::super::context::Binding {
        service: f.service.clone(),
        catalog: f.catalog.clone(),
        package_key: f.key.clone(),
    };
    let missing_pdf = f.directory.path().join("missing.pdf");
    let error = super::super::compile::export::export_pdf(
        store.clone(),
        binding.clone(),
        f.active.clone(),
        missing_pdf.to_string_lossy().into(),
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "compile_run_not_found");
    assert!(!missing_pdf.exists());
    let managed = store.manuscript_directory().join("export.pdf");
    fs::create_dir_all(managed.parent().unwrap()).unwrap();
    assert_eq!(
        super::super::compile::export::export_pdf(
            store.clone(),
            binding.clone(),
            f.active.clone(),
            managed.to_string_lossy().into(),
            false
        )
        .await
        .unwrap_err()
        .code,
        "invalid_input"
    );
    let managed_zip = store.manuscript_directory().join("export.zip");
    assert_eq!(
        export(&f, &store, &f.active, &managed_zip, false)
            .await
            .unwrap_err()
            .code,
        "invalid_input"
    );
    fs::write(&missing_pdf, b"existing external bytes").unwrap();
    assert_eq!(
        super::super::compile::export::export_pdf(
            store.clone(),
            binding,
            "missing-project".into(),
            missing_pdf.to_string_lossy().into(),
            false
        )
        .await
        .unwrap_err()
        .code,
        "file_conflict"
    );
    assert_eq!(fs::read(missing_pdf).unwrap(), b"existing external bytes");
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_import_finishes_once_after_the_original_waiter_leaves() {
    let f = Fixture::new();
    let store = files::store(&f);
    let input = input(&f);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let worker_gate = gate.clone();
    let (reached, waiting) = tokio::sync::oneshot::channel::<String>();
    let reached = std::sync::Mutex::new(Some(reached));
    {
        let db = store.connection().unwrap();
        db.create_scalar_function(
            "transfer_index_pause",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |context| {
                if let Some(reached) = reached.lock().unwrap().take() {
                    reached.send(context.get(0)?).unwrap();
                    let (released, signal) = &*worker_gate;
                    let mut released = released.lock().unwrap();
                    while !*released {
                        released = signal.wait(released).unwrap();
                    }
                }
                Ok(0)
            },
        )
        .unwrap();
        db.execute_batch("create trigger pause_transfer_index after insert on manuscript_files begin select transfer_index_pause(new.manuscript_id); end;").unwrap();
    }
    let task_store = store.clone();
    let service = f.service.clone();
    let catalog = f.catalog.clone();
    let key = f.key.clone();
    let task = tokio::spawn(async move {
        run_import(
            task_store,
            service,
            catalog,
            key,
            Call::ImportProject {
                title: "Retained import".into(),
                source_path: input.to_string_lossy().into(),
                requested_root: None,
            },
        )
        .await
    });
    let id = tokio::time::timeout(std::time::Duration::from_secs(30), waiting)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let (released, signal) = &*gate;
    *released.lock().unwrap() = true;
    signal.notify_all();
    // The accepted operation owns the same original bibliography IO lock until
    // all adoption and publication have settled, independently of the waiter.
    let settled_store = store.clone();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::task::spawn_blocking(move || {
            drop(settled_store.bibliography_io().unwrap());
        }),
    )
    .await
    .unwrap()
    .unwrap();
    let db = store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from manuscripts where title='Retained import'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "select count(*) from manuscript_files where manuscript_id=?1",
            [&id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        4
    );
    assert_eq!(
        db.query_row(
            "select count(*) from manuscript_bibliography_entries where manuscript_id=?1",
            [&id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    assert!(source(&store, &id).join("references.bib").is_file());
    f.service.shutdown().await.unwrap();
}

// macOS rejects the non-UTF-8 directory name before the transfer can run.
#[cfg(all(unix, not(target_os = "macos")))]
#[tokio::test]
#[ignore = "requires the actual independently built writing v2 package"]
async fn actual_package_keeps_os_paths_when_a_utf8_selection_has_a_non_utf8_parent() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let f = Fixture::new();
    let store = files::store(&f);
    let original = input(&f);
    let parent = f
        .directory
        .path()
        .join(std::ffi::OsString::from_vec(b"non-utf8-\xff".to_vec()));
    fs::create_dir(&parent).unwrap();
    fs::rename(original, parent.join("project")).unwrap();
    let alias = f.directory.path().join("selected-parent");
    symlink(&parent, &alias).unwrap();
    let selected = alias.join("project");
    assert!(selected.to_str().is_some());
    assert!(selected.canonicalize().unwrap().to_str().is_none());
    let imported = import(&f, &store, &selected, None).await.unwrap();
    let imported_main = fs::read(source(&store, &imported.manuscript.id).join("main.tex")).unwrap();
    assert!(
        String::from_utf8(imported_main.clone())
            .unwrap()
            .contains("\\documentclass")
    );

    assert_eq!(
        fs::read(selected.join("main.tex")).unwrap(),
        MAIN.as_bytes()
    );
    f.service.shutdown().await.unwrap();
}
