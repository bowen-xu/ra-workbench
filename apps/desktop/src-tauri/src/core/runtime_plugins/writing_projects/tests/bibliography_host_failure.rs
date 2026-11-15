//! The actual package exits while Core owns the original canonical write.
use super::bibliography_mutations::call;
use super::bibliography_runtime::list;
use super::files::{seed, store};
use super::*;
use crate::runtime_plugins::{
    literature_catalog::save::SaveOutcome,
    writing_projects::files::{FileRequest, bibliography::Call, operate_files},
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use tokio::time::{Duration, timeout};

const WAIT: Duration = Duration::from_secs(20);

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C07 package acceptance"]
async fn actual_package_host_exit_before_bibliography_preparation_preserves_original_and_recovers()
{
    let f = Fixture::new();
    let store = store(&f);
    let main = "\\cite{alpha}\\bibliography{refs}";
    seed(&store, &f.active, "main.tex", main);
    seed(
        &store,
        &f.active,
        "refs.bib",
        "@article{alpha,title={Original alpha},year={2024}}",
    );
    f.database.lock().unwrap().execute(
        "update manuscript_files set file_kind='bib' where manuscript_id=?1 and relative_path='refs.bib'",
        [&f.active],
    ).unwrap();
    let before = list(&f, &store).await.unwrap();
    assert_eq!(before.len(), 1);
    let entry = &before[0];
    let published = store.manuscript_files(&f.active).read("refs.bib").unwrap();
    let mut draft = entry.current.clone();
    draft["title"] = json!("Explicit save after preparation failure");
    let terminated_instance = f.service.terminate_active_host_for_test().await;
    let error = call(
        &f,
        &store,
        Call::Update {
            entry_id: entry.id.clone(),
            expected: entry.bibtex_entry.clone(),
            draft: draft.clone(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.code.as_str(),
        "plugin_host_input_failed" | "plugin_host_read_failed" | "plugin_host_exited"
    ));
    assert_eq!(error.save_outcome, Some(SaveOutcome::NotStarted));
    assert_eq!(
        error.instance_id.as_deref(),
        Some(terminated_instance.as_str())
    );
    assert_eq!(error.package_key.as_deref(), Some(f.key.as_str()));
    assert!(error.request_id.is_some());
    assert!(error.cleanup_error.is_none(), "{error:?}");
    assert_eq!(
        store.manuscript_files(&f.active).read("refs.bib").unwrap(),
        published
    );
    {
        let db = f.database.lock().unwrap();
        assert!(db.is_autocommit());
        let canonical: String = db.query_row(
            "select bibtex_entry from manuscript_bibliography_entries where manuscript_id=?1 and id=?2",
            rusqlite::params![f.active, entry.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(canonical, entry.bibtex_entry);
    }
    assert_eq!(list(&f, &store).await.unwrap(), before);
    let replacement = f
        .service
        .project_decision(
            f.key.clone(),
            research_workbench_native_sdk::writing_project::Input::PrepareStatus {
                manuscript_id: f.active.clone(),
            },
        )
        .await
        .unwrap();
    assert_ne!(replacement.instance_id, terminated_instance);
    let update = || Call::Update {
        entry_id: entry.id.clone(),
        expected: entry.bibtex_entry.clone(),
        draft: draft.clone(),
    };
    call(&f, &store, update()).await.unwrap();
    let saved = list(&f, &store).await.unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, entry.id);
    assert_eq!(saved[0].current["title"], draft["title"]);
    assert_eq!(saved[0].original_bibtex, entry.original_bibtex);
    assert_eq!(
        call(&f, &store, update()).await.unwrap_err().code,
        "file_conflict"
    );
    assert_eq!(list(&f, &store).await.unwrap(), saved);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an independently built writing v2 package; run the explicit C07 package acceptance"]
async fn actual_package_host_exit_after_canonical_update_recovers_publication_without_replaying_edit()
 {
    let f = Fixture::new();
    let store = store(&f);
    let main = "\\cite{alpha}\\bibliography{refs}";
    let other = "Unrelated synthetic chapter";
    let original = "@article{alpha,title={Original alpha},year={2024}}";
    seed(&store, &f.active, "main.tex", main);
    seed(&store, &f.active, "sections/other.tex", other);
    seed(&store, &f.active, "refs.bib", original);
    f.database.lock().unwrap().execute(
        "update manuscript_files set file_kind='bib' where manuscript_id=?1 and relative_path='refs.bib'",
        [&f.active],
    ).unwrap();
    let before = list(&f, &store).await.unwrap();
    assert_eq!(before.len(), 1);
    let entry = &before[0];
    let published_before = store.manuscript_files(&f.active).read("refs.bib").unwrap();
    let mut draft = entry.current.clone();
    draft["title"] = json!("Updated across actual host exit");

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    {
        let db = f.database.lock().unwrap();
        let observed = Arc::clone(&attempts);
        db.create_scalar_function(
            "test_bibliography_write_pause",
            0,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(WAIT).unwrap();
                Ok(0_i64)
            },
        )
        .unwrap();
        db.execute_batch(
            "create temp trigger test_bibliography_write_pause after update of bibtex_entry
             on manuscript_bibliography_entries begin select test_bibliography_write_pause(); end;",
        )
        .unwrap();
    }
    let client = tokio::spawn(Box::pin(operate_files(
        store.clone(),
        Arc::clone(&f.service),
        Arc::clone(&f.catalog),
        f.key.clone(),
        f.active.clone(),
        FileRequest::Bibliography {
            call: Call::Update {
                entry_id: entry.id.clone(),
                expected: entry.bibtex_entry.clone(),
                draft: draft.clone(),
            },
        },
    )));
    timeout(
        WAIT,
        tokio::task::spawn_blocking(move || entered_rx.recv_timeout(WAIT)),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    let terminated_instance = f.service.terminate_active_host_for_test().await;
    release_tx.send(()).unwrap();
    let error = timeout(WAIT, client).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error.code, "plugin_host_input_failed");
    assert_eq!(error.plugin_id, "writing_projects");
    assert_eq!(error.package_key.as_deref(), Some(f.key.as_str()));
    assert_eq!(
        error.instance_id.as_deref(),
        Some(terminated_instance.as_str())
    );
    assert!(error.request_id.is_some());
    assert_eq!(error.save_outcome, Some(SaveOutcome::Unknown));
    assert!(error.cleanup_error.is_none());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    {
        let db = f.database.lock().unwrap();
        assert!(db.is_autocommit());
        let canonical: String = db.query_row(
            "select bibtex_entry from manuscript_bibliography_entries where manuscript_id=?1 and id=?2",
            rusqlite::params![f.active, entry.id],
            |row| row.get(0),
        ).unwrap();
        assert!(canonical.contains("Updated across actual host exit"));
    }
    assert_eq!(
        store.manuscript_files(&f.active).read("refs.bib").unwrap(),
        published_before
    );

    // An explicit read starts the replacement and publishes the committed rows.
    let recovered = list(&f, &store).await.unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].id, entry.id);
    assert_eq!(recovered[0].current["title"], draft["title"]);
    assert_eq!(recovered[0].imported, entry.imported);
    assert_eq!(recovered[0].original_bibtex, entry.original_bibtex);
    let replacement = f
        .service
        .project_decision(
            f.key.clone(),
            research_workbench_native_sdk::writing_project::Input::PrepareStatus {
                manuscript_id: f.active.clone(),
            },
        )
        .await
        .unwrap();
    assert_ne!(replacement.instance_id, terminated_instance);
    let published = store.manuscript_files(&f.active).read("refs.bib").unwrap();
    assert!(published.0.contains("Updated across actual host exit"));
    let stale = call(
        &f,
        &store,
        Call::Update {
            entry_id: entry.id.clone(),
            expected: entry.bibtex_entry.clone(),
            draft,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(stale.code, "file_conflict");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(list(&f, &store).await.unwrap(), recovered);
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
            .read("sections/other.tex")
            .unwrap()
            .0,
        other
    );

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
    let crate::runtime_plugins::writing_projects::files::FileResult::Bibliography(reopened) =
        operate_files(
            store.clone(),
            Arc::clone(&service),
            catalog,
            key,
            active.clone(),
            FileRequest::Bibliography {
                call: Call::List {
                    include_unused_automatic: false,
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("reopened bibliography");
    };
    assert_eq!(reopened, recovered);
    assert_eq!(
        store.manuscript_files(&active).read("refs.bib").unwrap(),
        published
    );
    assert_eq!(
        store.manuscript_files(&active).read("main.tex").unwrap().0,
        main
    );
    assert_eq!(
        store
            .manuscript_files(&active)
            .read("sections/other.tex")
            .unwrap()
            .0,
        other
    );
    service.shutdown().await.unwrap();
}
