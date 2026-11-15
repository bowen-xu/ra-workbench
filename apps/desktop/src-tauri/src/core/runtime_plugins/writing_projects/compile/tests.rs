//! Complete formal compile consumer against the real independent package and TeX.
use super::*;
use crate::{
    compile_activity::CompileManager,
    runtime_plugins::writing_projects::{
        context::Binding,
        tests::{
            Fixture,
            files::{seed, store},
        },
    },
    storage::AppStore,
};
use research_workbench_native_sdk::writing_compile::{CompileOptions, CompileRun};
use std::{fs, time::Duration};

fn binding(f: &Fixture) -> Binding {
    Binding {
        service: f.service.clone(),
        catalog: f.catalog.clone(),
        package_key: f.key.clone(),
    }
}

async fn settled(f: &Fixture, store: &AppStore, run_id: &str) -> CompileRun {
    let id = run_id.to_owned();
    tokio::time::timeout(Duration::from_secs(150), async {
        loop {
            let id = id.clone();
            let run = read::task(store.clone(), binding(f), move |store, domain, sql| {
                let run = {
                    let db = store
                        .connection()
                        .map_err(|cause| domain.error(cause.into()))?;
                    repository::get_run(&sql, &db, &id).map_err(|cause| domain.error(cause))?
                };
                domain.run(run)
            })
            .await
            .unwrap();
            if !matches!(run.status.as_str(), "queued" | "running") {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .expect("original bounded compile must settle")
}

const SOURCE: &str = "\\documentclass{article}\n\\begin{document}\n\\section{Runtime compile}\n\\input{sections/intro}\n\\end{document}\n";

#[tokio::test]
#[ignore = "requires actual independent writing package and installed TeX; explicit C05 acceptance"]
async fn actual_package_compiles_reads_pdf_locates_and_rejects_stale_input() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", SOURCE);
    seed(
        &store,
        &f.active,
        "sections/intro.tex",
        "Independent runtime source.\n",
    );
    let manager = CompileManager::default();
    let queued = start::start(
        store.clone(),
        binding(&f),
        manager.clone(),
        f.active.clone(),
        "pdflatex".into(),
        Some(CompileOptions {
            draft: true,
            stop_on_first_error: false,
            from_scratch: true,
        }),
    )
    .await
    .unwrap();
    assert_eq!(queued.status, "queued");
    let run = settled(&f, &store, &queued.id).await;
    assert_eq!(run.status, "succeeded", "{run:?}");
    let destination = f.directory.path().join("compiled.pdf");
    let exported = export::export_pdf(
        store.clone(),
        binding(&f),
        f.active.clone(),
        destination.to_string_lossy().into(),
        false,
    )
    .await
    .unwrap();
    let original_pdf = fs::read(&destination).unwrap();
    assert!(original_pdf.starts_with(b"%PDF-"));
    assert_eq!(exported.byte_size, original_pdf.len() as u64);
    assert_eq!(exported.file_count, 1);
    assert_eq!(
        export::export_pdf(
            store.clone(),
            binding(&f),
            f.active.clone(),
            destination.to_string_lossy().into(),
            false
        )
        .await
        .unwrap_err()
        .code,
        "file_conflict"
    );
    let numbered = export::export_pdf(
        store.clone(),
        binding(&f),
        f.active.clone(),
        destination.to_string_lossy().into(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(
        std::path::Path::new(&numbered.destination_path),
        f.directory
            .path()
            .canonicalize()
            .unwrap()
            .join("compiled (1).pdf")
    );
    assert_eq!(fs::read(numbered.destination_path).unwrap(), original_pdf);
    assert_eq!(fs::read(&destination).unwrap(), original_pdf);
    let id = run.id.clone();
    read::task(store.clone(), binding(&f), move |store, domain, sql| {
        assert!(artifacts::current(&store, &sql, &id).unwrap());
        assert!(
            artifacts::pdf(&store, &sql, &id)
                .unwrap()
                .starts_with(b"%PDF-")
        );
        let locations = synctex::forward(&store, &domain, &sql, &id, "sections/intro.tex", 1, 0)?;
        assert!(!locations.is_empty());
        let first = &locations[0];
        let inverse = synctex::inverse(&store, &domain, &sql, &id, first.page, first.x, first.y)?;
        assert!(
            inverse
                .iter()
                .any(|location| location.relative_path == "sections/intro.tex")
        );
        let db = store.connection().unwrap();
        assert_eq!(
            repository::latest_run(&sql, &db, &run.manuscript_id)
                .unwrap()
                .unwrap()
                .id,
            id
        );
        assert_eq!(
            repository::latest_pdf_run(&sql, &db, &run.manuscript_id)
                .unwrap()
                .unwrap()
                .id,
            id
        );
        Ok(())
    })
    .await
    .unwrap();
    store
        .manuscript_files(&f.active)
        .save("sections/intro.tex", "Changed after compile.\n", None)
        .unwrap();
    let id = queued.id.clone();
    read::task(store.clone(), binding(&f), move |store, domain, sql| {
        assert!(!artifacts::current(&store, &sql, &id).unwrap());
        assert!(
            artifacts::pdf(&store, &sql, &id)
                .unwrap()
                .starts_with(b"%PDF-")
        );
        assert_eq!(
            synctex::forward(&store, &domain, &sql, &id, "main.tex", 3, 0)
                .unwrap_err()
                .code,
            "synctex_unavailable"
        );
        assert_eq!(
            synctex::inverse(&store, &domain, &sql, &id, 1, f64::NAN, 0.0)
                .unwrap_err()
                .code,
            "invalid_input"
        );
        Ok(())
    })
    .await
    .unwrap();
    let repeated = start::start(
        store.clone(),
        binding(&f),
        manager,
        f.active.clone(),
        "pdflatex".into(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(settled(&f, &store, &repeated.id).await.status, "succeeded");
    f.service.shutdown().await.unwrap();
    let reopened_service = std::sync::Arc::new(
        crate::runtime_plugins::native::NativePreviewService::new(f.catalog.clone()),
    );
    let id = repeated.id;
    read::task(
        store,
        Binding {
            service: reopened_service.clone(),
            ..binding(&f)
        },
        move |store, domain, sql| {
            let run = {
                let db = store.connection().unwrap();
                repository::get_run(&sql, &db, &id).unwrap()
            };
            assert_eq!(domain.run(run)?.status, "succeeded");
            assert!(
                artifacts::pdf(&store, &sql, &id)
                    .unwrap()
                    .starts_with(b"%PDF-")
            );
            assert!(
                !synctex::forward(&store, &domain, &sql, &id, "sections/intro.tex", 1, 0)?
                    .is_empty()
            );
            Ok(())
        },
    )
    .await
    .unwrap();
    reopened_service.shutdown().await.unwrap();
    f.database.shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package and installed TeX; explicit C05 acceptance"]
async fn actual_package_keeps_failed_pdf_cancels_original_process_and_releases_owner() {
    let f = Fixture::new();
    let store = store(&f);
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\documentclass{article}\n\\begin{document}\nVisible output.\\UndefinedC05Command\n\\end{document}\n",
    );
    let manager = CompileManager::default();
    let failed = start::start(
        store.clone(),
        binding(&f),
        manager.clone(),
        f.active.clone(),
        "pdflatex".into(),
        None,
    )
    .await
    .unwrap();
    let failed = settled(&f, &store, &failed.id).await;
    assert_eq!(failed.status, "failed", "{failed:?}");
    assert!(
        failed
            .diagnostics
            .iter()
            .any(|item| item.severity == "error")
    );
    let failed_id = failed.id.clone();
    read::task(store.clone(), binding(&f), move |store, domain, sql| {
        assert!(
            artifacts::pdf(&store, &sql, &failed_id)
                .unwrap()
                .starts_with(b"%PDF-")
        );
        let db = store.connection().unwrap();
        let latest = repository::latest_pdf_run(&sql, &db, &failed.manuscript_id)
            .unwrap()
            .unwrap();
        assert_eq!(latest.id, failed_id);
        drop(db);
        let terminal =
            artifacts::cancel(&store, &sql, &CompileManager::default(), &failed_id).unwrap_err();
        assert_eq!(domain.error(terminal).code, "compile_run_not_found");
        Ok(())
    })
    .await
    .unwrap();
    store.manuscript_files(&f.active).save("main.tex", "\\documentclass{article}\n\\begin{document}\n\\newcount\\counter\\loop\\advance\\counter by1\\iftrue\\repeat\n\\end{document}\n", None).unwrap();
    let queued = start::start(
        store.clone(),
        binding(&f),
        manager.clone(),
        f.active.clone(),
        "pdflatex".into(),
        None,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let state: String = f
                .database
                .lock()
                .unwrap()
                .query_row(
                    "select status from manuscript_compile_runs where id=?1",
                    [&queued.id],
                    |row| row.get(0),
                )
                .unwrap();
            if state == "running" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let id = queued.id.clone();
    read::task(store.clone(), binding(&f), move |store, domain, sql| {
        artifacts::cancel(&store, &sql, &manager, &id).map_err(|cause| domain.error(cause))
    })
    .await
    .unwrap();
    let cancelled = settled(&f, &store, &queued.id).await;
    assert_eq!(cancelled.status, "cancelled");
    assert!(
        cancelled
            .artifacts
            .iter()
            .all(|artifact| artifact.kind != "pdf")
    );
    let old_pdf = failed.id;
    let manuscript = f.active.clone();
    read::task(store.clone(), binding(&f), move |store, _domain, sql| {
        let db = store.connection().unwrap();
        assert_eq!(
            repository::latest_pdf_run(&sql, &db, &manuscript)
                .unwrap()
                .unwrap()
                .id,
            old_pdf
        );
        Ok(())
    })
    .await
    .unwrap();
    f.service.shutdown().await.unwrap();
    f.database.shutdown().unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing package and installed TeX; explicit C05 acceptance"]
async fn actual_package_rejects_missing_selection_and_shutdown_drains_accepted_compile() {
    let f = Fixture::new();
    let store = store(&f);
    seed(&store, &f.active, "main.tex", SOURCE);
    seed(
        &store,
        &f.active,
        "sections/intro.tex",
        "Accepted before shutdown.\n",
    );
    let manager = CompileManager::default();
    let missing = Binding {
        package_key: "0".repeat(64),
        ..binding(&f)
    };
    let rejection = start::start(
        store.clone(),
        missing,
        manager.clone(),
        f.active.clone(),
        "pdflatex".into(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(rejection.code, "plugin_host_package_unavailable");
    assert_eq!(
        f.database
            .lock()
            .unwrap()
            .query_row("select count(*) from manuscript_compile_runs", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    let invalid = start::start(
        store.clone(),
        binding(&f),
        manager.clone(),
        "missing manuscript".into(),
        "arbitrary".into(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.code, "invalid_engine");
    assert!(invalid.instance_id.is_some() && invalid.request_id.is_some());
    let queued = start::start(
        store,
        binding(&f),
        manager.clone(),
        f.active.clone(),
        "pdflatex".into(),
        None,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(150), f.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    let state: String = f
        .database
        .lock()
        .unwrap()
        .query_row(
            "select status from manuscript_compile_runs where id=?1",
            [&queued.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        matches!(state.as_str(), "failed" | "succeeded"),
        "shutdown left {state}"
    );
    manager.with_idle_manuscript(&f.active, || Ok(())).unwrap();
    f.database.shutdown().unwrap();
}
