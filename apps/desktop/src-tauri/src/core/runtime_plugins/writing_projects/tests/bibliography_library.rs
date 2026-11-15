//! Both independent packages use the real Core connection and original files.
use super::bibliography_runtime::list;
use super::files::{seed, store};
use super::*;
use crate::runtime_plugins::{
    literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
    writing_projects::files::{FileRequest, FileResult, bibliography::Call, operate_files},
};
use rusqlite::params;
use std::{fs, path::Path};

fn copy(source: &Path, target: &Path) {
    assert!(fs::symlink_metadata(source).unwrap().is_dir());
    fs::create_dir(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(!kind.is_symlink());
        let target = target.join(entry.file_name());
        if kind.is_dir() {
            copy(&entry.path(), &target);
        } else {
            assert!(kind.is_file());
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

pub(super) fn bind(f: &mut Fixture) -> (tempfile::TempDir, Arc<LiteratureTraceService>, String) {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    let literature = fs::canonicalize(
        std::env::var_os("RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE")
            .expect("Set the actual independent literature package"),
    )
    .unwrap();
    let writing =
        fs::canonicalize(std::env::var_os("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES").unwrap())
            .unwrap();
    assert!(literature.starts_with(repository.join(".tmp")));
    assert!(writing.starts_with(repository.join(".tmp")));
    let directory = tempfile::tempdir_in(repository.join(".tmp")).unwrap();
    copy(&writing.join("package"), &directory.path().join("writing"));
    copy(&literature, &directory.path().join("literature"));
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(directory.path().into())));
    let listing = catalog.discover().unwrap();
    assert_eq!(listing.packages.len(), 2);
    let key = |id| {
        listing
            .packages
            .iter()
            .find(|p| p.manifest["id"] == id)
            .unwrap()
            .package_key
            .clone()
    };
    f.key = key("writing_projects");
    let literature_key = key("literature_library");
    f.service = Arc::new(NativePreviewService::new(Arc::clone(&catalog)));
    f.catalog = Arc::clone(&catalog);
    (
        directory,
        Arc::new(LiteratureTraceService::new(catalog)),
        literature_key,
    )
}

#[tokio::test]
#[ignore = "requires actual independent writing and literature packages; run the explicit C05 package acceptance"]
async fn actual_packages_bibliography_adoption_and_refresh_use_live_literature_metadata() {
    let mut f = Fixture::new();
    let (_packages, literature, key) = bind(&mut f);
    let store = store(&f).with_native_note_trace(NoteTraceBinding {
        service: Arc::clone(&literature),
        package_key: key.clone(),
    });
    {
        let db = f.database.lock().unwrap();
        db.execute("insert into creators(id,family_name,given_name,literal_name) values('author','Example','A','')", []).unwrap();
        for (id, title, year) in [
            ("doi-item", "Live DOI title", 2026),
            ("fallback-item", "Fallback title", 2024),
            ("title-item", "Title only", 2024),
        ] {
            db.execute("insert into bibliographic_items(id,item_type,title,issued_year,created_at,updated_at) values(?1,'journalArticle',?2,?3,'now','now')", params![id,title,year]).unwrap();
            db.execute("insert into item_creators(item_id,creator_id,role,position) values(?1,'author','author',0)", [id]).unwrap();
        }
        db.execute("insert into identifiers(id,item_id,kind,normalized_value,display_value) values('doi','doi-item','doi','10.1234/exact','10.1234/exact')", []).unwrap();
    }
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\documentclass{article}\n\\begin{document}\\cite{exact,fallback,title}\\bibliography{refs}\\end{document}",
    );
    let original = "@article{exact,title={Imported DOI title},year={2020},author={Other, B},doi={10.1234/exact}}\n@article{fallback,title={Fallback title},year={2024},author={Example, A},doi={10.1234/missing}}\n@article{title,title={Title only},year={2024},author={Example, A}}\n";
    seed(&store, &f.active, "refs.bib", original);
    f.database.lock().unwrap().execute("update manuscript_files set file_kind='bib' where manuscript_id=?1 and relative_path='refs.bib'", [&f.active]).unwrap();
    let before = list(&f, &store).await.unwrap();
    assert_eq!(before.len(), 3);
    for (citation_key, id, title) in [
        ("exact", "doi-item", "Live DOI title"),
        ("fallback", "fallback-item", "Fallback title"),
        ("title", "title-item", "Title only"),
    ] {
        let entry = before
            .iter()
            .find(|entry| entry.citation_key == citation_key)
            .unwrap();
        assert_eq!(entry.library_item_id.as_deref(), Some(id));
        assert_eq!(entry.current["title"], title);
        assert_eq!(entry.library_item.as_ref().unwrap()["title"], title);
    }
    let exact = before
        .iter()
        .find(|entry| entry.citation_key == "exact")
        .unwrap();
    assert_eq!(exact.imported["title"], "Imported DOI title");
    f.database
        .lock()
        .unwrap()
        .execute(
            "update bibliographic_items set title='Updated live title' where id='doi-item'",
            [],
        )
        .unwrap();
    let observed = list(&f, &store).await.unwrap();
    let observed = observed
        .iter()
        .find(|entry| entry.citation_key == "exact")
        .unwrap();
    assert_eq!(observed.current["title"], "Live DOI title");
    assert_eq!(
        observed.library_item.as_ref().unwrap()["title"],
        "Updated live title"
    );
    let result = operate_files(
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
    .unwrap();
    assert!(matches!(result, FileResult::BibliographyStatus(status) if status.status == "ready"));
    let refreshed = list(&f, &store).await.unwrap();
    let refreshed = refreshed
        .iter()
        .find(|entry| entry.citation_key == "exact")
        .unwrap();
    assert_eq!(refreshed.id, exact.id);
    assert_eq!(refreshed.imported, exact.imported);
    assert_eq!(refreshed.original_bibtex, exact.original_bibtex);
    assert_eq!(refreshed.current["title"], "Updated live title");
    let retained: String = f.database.lock().unwrap().query_row("select original_content from manuscript_bibliography_files where manuscript_id=?1 and relative_path='refs.bib'", [&f.active], |row| row.get(0)).unwrap();
    assert_eq!(retained, original);
    assert!(
        store
            .manuscript_files(&f.active)
            .read("refs.bib")
            .unwrap()
            .0
            .contains("Updated live title")
    );
    f.database
        .lock()
        .unwrap()
        .execute(
            "update bibliographic_items set archived_at='now' where id='doi-item'",
            [],
        )
        .unwrap();
    let missing = list(&f, &store).await.unwrap_err();
    assert_eq!(missing.code, "bibliography_import");
    assert_eq!(missing.plugin_id, "literature_library");
    assert_eq!(missing.package_key.as_deref(), Some(key.as_str()));
    assert!(missing.instance_id.is_some() && missing.request_id.is_some());
    f.database
        .lock()
        .unwrap()
        .execute(
            "update bibliographic_items set archived_at=null where id='doi-item'",
            [],
        )
        .unwrap();
    assert_eq!(list(&f, &store).await.unwrap().len(), 3);
    {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        let db = f.database.lock().unwrap();
        db.authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "item_creators",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    }
    let failed = list(&f, &store).await.unwrap_err();
    f.database
        .lock()
        .unwrap()
        .authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>);
    assert_eq!(failed.code, "bibliography_import");
    assert_eq!(failed.plugin_id, "literature_library");
    assert_eq!(failed.package_key.as_deref(), Some(key.as_str()));
    assert_eq!(
        failed.data_error.as_ref().unwrap().code,
        "plugin_data_sql_failed"
    );
    assert_eq!(list(&f, &store).await.unwrap().len(), 3);
    f.service.shutdown().await.unwrap();
    literature.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent writing and literature packages; run the explicit C05 package acceptance"]
async fn actual_packages_sync_keeps_skips_and_ordered_partial_updates_before_rejection() {
    use research_workbench_native_sdk::writing_bibliography::BibliographyEntryInput;
    let mut f = Fixture::new();
    let (_packages, literature, key) = bind(&mut f);
    let store = store(&f).with_native_note_trace(NoteTraceBinding {
        service: Arc::clone(&literature),
        package_key: key,
    });
    seed(
        &store,
        &f.active,
        "main.tex",
        "\\documentclass{article}\n\\begin{document}\\cite{first,second}\\bibliography{refs}\\end{document}",
    );
    {
        let db = f.database.lock().unwrap();
        for (id, title, key) in [
            ("one", "First library", "first"),
            ("two", "Second library", "second"),
        ] {
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle',?2,'now','now')", params![id,title]).unwrap();
            db.execute("insert into citation_bindings(manuscript_id,literature_item_id,source_item_id,citation_key,bibtex_entry,state,pinned_at,updated_at) values(?1,?2,?2,?3,?4,'active','now','now')", params![f.active,id,key,format!("@article{{{key},title={{Before}}}}")]).unwrap();
        }
    }
    let input = |id: &str, text: &str| BibliographyEntryInput {
        literature_item_id: id.into(),
        requested_key: Some("ignored-originally".into()),
        bibtex_entry: text.into(),
    };
    let sync = |entries| {
        operate_files(
            store.clone(),
            Arc::clone(&f.service),
            Arc::clone(&f.catalog),
            f.key.clone(),
            f.active.clone(),
            FileRequest::Bibliography {
                call: Call::Sync { entries },
            },
        )
    };
    let failed = sync(vec![
        input("unbound", "invalid, skipped"),
        input("one", "@article{incoming,title={First candidate}}"),
        input("two", "invalid"),
    ])
    .await
    .unwrap_err();
    assert_eq!(failed.code, "invalid_bibtex_entry");
    assert!(failed.instance_id.is_some() && failed.request_id.is_some());
    {
        let db = f.database.lock().unwrap();
        let text = |id| {
            db.query_row("select bibtex_entry from citation_bindings where manuscript_id=?1 and source_item_id=?2", params![f.active,id], |row| row.get::<_,String>(0)).unwrap()
        };
        assert_eq!(text("one"), "@article{first,title={First candidate}}");
        assert_eq!(text("two"), "@article{second,title={Before}}");
    }
    let result = sync(vec![input(
        "two",
        "@article{incoming,title={Second candidate}}",
    )])
    .await
    .unwrap();
    assert!(matches!(result, FileResult::BibliographyStatus(status) if status.status == "ready"));
    let entries = list(&f, &store).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry.citation_key == "first" && entry.current["title"] == "First library"));
    assert!(
        entries
            .iter()
            .any(|entry| entry.citation_key == "second"
                && entry.current["title"] == "Second library")
    );
    f.service.shutdown().await.unwrap();
    literature.shutdown().await.unwrap();
}
