use super::*;
use crate::runtime_plugins::writing_projects::citation_metadata::read_metadata;
use research_workbench_native_sdk::writing_project::{
    BibliographyStatus, CitationMetadataKind, Decision,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

async fn read(f: &Fixture, id: &str, kind: CitationMetadataKind) -> NativeResult<Decision> {
    read_metadata(
        Arc::clone(&f.database),
        &f.service,
        Arc::clone(&f.catalog),
        f.key.clone(),
        id.into(),
        kind,
    )
    .await
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_citation_metadata_preserves_defaults_order_full_bytes_and_read_only_owner()
{
    let f = Fixture::new();
    let changes = f.database.lock().unwrap().total_changes();
    assert_eq!(
        read(
            &f,
            &format!(" {} ", f.active),
            CitationMetadataKind::BibliographyStatus
        )
        .await
        .unwrap(),
        Decision::BibliographyStatus {
            status: BibliographyStatus {
                relative_path: "references.bib".into(),
                content_hash: None,
                generated_at: None,
                status: "ready".into(),
                warnings: vec![],
            }
        }
    );
    assert_eq!(f.database.lock().unwrap().total_changes(), changes);
    {
        let db = f.database.lock().unwrap();
        for (source, key, entry) in [
            ("lower", "aKey", ""),
            ("upper", "ZKey", "@article{ZKey,\r\n title={原文}}，"),
        ] {
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle',?1,'now','now')",[source]).unwrap();
            db.execute("insert into citation_bindings(manuscript_id,source_item_id,citation_key,bibtex_entry,state,pinned_at,updated_at) values(?1,?2,?3,?4,'active','pinned','updated')",rusqlite::params![f.active,source,key,entry]).unwrap();
        }
        db.execute("insert into managed_bibliographies(manuscript_id,relative_path,content_hash,generated_at,status) values(?1,'refs/原文.bib',null,null,'conflict')",[&f.active]).unwrap();
    }
    let changes = f.database.lock().unwrap().total_changes();
    let Decision::CitationBindings { items } = read(&f, &f.active, CitationMetadataKind::Bindings)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        items
            .iter()
            .map(|item| item.citation_key.as_str())
            .collect::<Vec<_>>(),
        ["ZKey", "aKey"]
    );
    assert_eq!(items[0].bibtex_entry, "@article{ZKey,\r\n title={原文}}，");
    assert_eq!(items[0].pinned_at, "pinned");
    let Decision::BibliographyStatus { status } =
        read(&f, &f.active, CitationMetadataKind::BibliographyStatus)
            .await
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(status.relative_path, "refs/原文.bib");
    assert_eq!(status.status, "conflict");
    assert!(
        status.content_hash.is_none()
            && status.generated_at.is_none()
            && status.warnings.is_empty()
    );
    {
        let db = f.database.lock().unwrap();
        assert_eq!(db.total_changes(), changes);
        assert!(db.is_autocommit());
    }
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_citation_metadata_keeps_missing_manuscript_before_related_sql_error() {
    let f = Fixture::new();
    f.database
        .lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "citation_bindings" | "managed_bibliographies",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    for kind in [
        CitationMetadataKind::Bindings,
        CitationMetadataKind::BibliographyStatus,
    ] {
        let missing = read(&f, " missing ", kind).await.unwrap_err();
        assert_eq!(missing.code, "not_found");
        assert_eq!(missing.message, "稿件不存在");
        assert_eq!(missing.package_key.as_deref(), Some(f.key.as_str()));
        assert!(missing.instance_id.is_some() && missing.request_id.is_some());
        let denied = read(&f, &f.active, kind).await.unwrap_err();
        assert_eq!(denied.code, "storage_error");
        assert!(denied.data_error.is_some());
    }
    f.database
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    f.service.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_citation_metadata_rechecks_related_rows_after_releasing_database_for_native()
 {
    for (kind, table, sql, resource) in [
        (
            CitationMetadataKind::Bindings,
            "citation_bindings",
            "insert into citation_bindings(manuscript_id,source_item_id,citation_key,bibtex_entry,state,pinned_at,updated_at) values(?1,'source','Key','full bibtex','active','pinned','updated')",
            "writing.projects.citation-bindings",
        ),
        (
            CitationMetadataKind::BibliographyStatus,
            "managed_bibliographies",
            "insert into managed_bibliographies(manuscript_id,relative_path,content_hash,generated_at,status) values(?1,'refs.bib',null,null,'ready')",
            "writing.projects.bibliography-status",
        ),
    ] {
        let f = Fixture::new();
        let captured = Arc::new(tokio::sync::Notify::new());
        let (release, released) = mpsc::channel();
        {
            let db = f.database.lock().unwrap();
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('source','journalArticle','Synthetic','now','now')", []).unwrap();
            let signal = Arc::clone(&captured);
            let seen = AtomicBool::new(false);
            db.authorizer(Some(move |context: AuthContext<'_>| {
                if matches!(context.action, AuthAction::Read {table_name, ..} if table_name == table)
                    && !seen.swap(true, Ordering::SeqCst) {
                    signal.notify_one();
                    if released.recv_timeout(Duration::from_secs(90)).is_err() {return Authorization::Deny;}
                }
                Authorization::Allow
            }));
        }
        let database = Arc::clone(&f.database);
        let service = Arc::clone(&f.service);
        let catalog = Arc::clone(&f.catalog);
        let key = f.key.clone();
        let id = f.active.clone();
        let reading =
            tokio::spawn(
                async move { read_metadata(database, &service, catalog, key, id, kind).await },
            );
        tokio::time::timeout(Duration::from_secs(90), captured.notified())
            .await
            .unwrap();
        let queued = Arc::new(tokio::sync::Notify::new());
        let signal = Arc::clone(&queued);
        let database = Arc::clone(&f.database);
        let id = f.active.clone();
        let writer = tokio::task::spawn_blocking(move || {
            signal.notify_one();
            // This owner must be available during cold Native initialization.
            database.lock().unwrap().execute(sql, [id]).unwrap();
        });
        tokio::time::timeout(Duration::from_secs(90), queued.notified())
            .await
            .unwrap();
        release.send(()).unwrap();
        writer.await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(90), reading)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "plugin_data_snapshot_changed");
        let diagnostic = error.data_error.unwrap();
        assert_eq!(diagnostic.resource_id, resource);
        assert_eq!(diagnostic.phase, "recheck");
        assert!(error.instance_id.is_some() && error.request_id.is_some());
        f.database
            .lock()
            .unwrap()
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        f.service.shutdown().await.unwrap();
    }
}
