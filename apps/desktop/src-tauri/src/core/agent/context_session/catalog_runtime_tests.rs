//! Original directory/search/grant entry points with a real accepted SQL package.
use super::*;
use crate::{
    agent::context_catalog::PaperCatalogReader,
    runtime_plugins::{
        RuntimePluginCatalog,
        literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
    },
};
use rusqlite::params;
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};

fn package(catalog_read: bool) -> (tempfile::TempDir, Arc<LiteratureTraceService>, String) {
    let root = tempfile::tempdir().unwrap();
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(root.path().into())));
    // The service predates installation; acceptance must refresh this exact key.
    let service = Arc::new(LiteratureTraceService::new(Arc::clone(&catalog)));
    let dir = root.path().join("literature");
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(
        dir.join("host"),
        "not executable; inventory never starts a host",
    )
    .unwrap();
    let mut provides = vec![json!({"id":"literature.note-trace-decision","version":1})];
    if catalog_read {
        provides.push(json!({"id":"literature.catalog-read","version":1}));
    }
    fs::write(dir.join("manifest.json"), serde_json::to_vec(&json!({
        "formatVersion":2,"id":"literature_library","version":"0.1.0","apiVersion":1,
        "dependencies":{"required":[],"optional":[]},"consumes":{"required":[],"optional":[]},
        "provides":provides,
        "native":{"protocolVersion":2,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"entry":"host"}
    })).unwrap()).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for name in ["papers", "pdf-attachments"] {
        let file = format!("context-catalog-{name}.v1.sql");
        fs::copy(source.join(&file), dir.join("data").join(file)).unwrap();
    }
    let key = catalog.discover().unwrap().packages.remove(0).package_key;
    (root, service, key)
}

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'fixture.pdf','application/pdf',1,'now')",["a".repeat(64)]).unwrap();
    db
}

fn paper(db: &Connection, id: &str, title: &str, updated: &str) {
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'document',?2,'now',?3)",params![id,title,updated]).unwrap();
}

fn store(
    db: Connection,
    root: &Path,
    service: Arc<LiteratureTraceService>,
    key: String,
) -> AppStore {
    AppStore::for_test(db, root.join("objects")).with_native_note_trace(NoteTraceBinding {
        service,
        package_key: key,
    })
}

#[tokio::test]
async fn runtime_catalog_uses_original_search_groups_bounds_order_and_configuration() {
    let (root, service, key) = package(true);
    let db = database();
    for index in 0..65 {
        paper(
            &db,
            &format!("paper-{index:02}"),
            &format!("Needle 画像 α {index:02}"),
            &format!("{index:03}"),
        );
    }
    db.execute(
        "update bibliographic_items set trashed_at='now' where id='paper-64'",
        [],
    )
    .unwrap();
    db.execute(
        "update bibliographic_items set archived_at='now' where id='paper-63'",
        [],
    )
    .unwrap();
    for (id, name, kind, created) in [
        ("pdf-b", "Second α.pdf", "pdf", "a"),
        ("pdf-a", "First 汉字.pdf", "pdf", "a"),
        ("pdf-later", "Later.pdf", "pdf", "b"),
        ("image", "Image.png", "image", "0"),
    ] {
        db.execute("insert into attachments(id,item_id,content_sha256,display_name,attachment_kind,created_at) values(?1,'paper-63',?2,?3,?4,?5)",params![id,"a".repeat(64),name,kind,created]).unwrap();
    }
    let conversation = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Context catalog fixture')",
        [conversation.to_string()],
    )
    .unwrap();
    let changes = db.total_changes();
    let store = store(db, root.path(), service, key);
    let rows = search_catalog(&store, "Needle".into()).await.unwrap();
    assert_eq!(rows.len(), 60);
    assert_eq!(
        rows.iter()
            .map(|selection| match &selection.resource {
                sources::Resource::PaperGroup { resource_id, .. } => {
                    (resource_id.clone(), selection.label.clone())
                }
                _ => panic!("paper group"),
            })
            .collect::<Vec<_>>(),
        (4..=63)
            .rev()
            .map(|index| {
                (
                    format!("paper-{index:02}"),
                    format!("Needle 画像 α {index:02}"),
                )
            })
            .collect::<Vec<_>>()
    );
    let sources::Resource::PaperGroup {
        resource_id, parts, ..
    } = &rows[0].resource
    else {
        panic!("original paper group");
    };
    assert_eq!(resource_id, "paper-63");
    let pdfs = parts
        .iter()
        .filter_map(|part| match &part.resource {
            sources::Resource::Paper {
                attachment_id: Some(id),
                page_index: None,
                ..
            } => {
                assert_eq!(part.access, sources::Access::ReadOnly);
                Some((id.as_str(), part.label.as_str()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pdfs,
        [
            ("pdf-a", "First 汉字.pdf"),
            ("pdf-b", "Second α.pdf"),
            ("pdf-later", "Later.pdf")
        ]
    );
    assert_eq!(store.connection().unwrap().total_changes(), changes);
    assert!(
        search_catalog(&store, "no match".into())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        search_catalog(&store, "First 汉字".into())
            .await
            .unwrap()
            .len(),
        1
    );
    let saved = configuration::save_configuration(
        &store,
        configuration::SaveRequest {
            conversation_id: conversation,
            expected_revision: 0,
            state: json!({"schemaVersion":1,"persistent":[rows[0]],"temporary":[]}),
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        saved.state["persistent"][0],
        serde_json::to_value(&rows[0]).unwrap()
    );
    assert_eq!(
        configuration::read_in(&store.connection().unwrap(), conversation)
            .unwrap()
            .unwrap()
            .state,
        saved.state
    );
}

#[tokio::test]
async fn runtime_catalog_missing_selection_never_falls_back_and_query_budget_precedes_acceptance() {
    let (root, service, _) = package(true);
    let db = database();
    paper(&db, "paper", "Visible via static", "now");
    let key = "b".repeat(64);
    let store = store(db, root.path(), service, key.clone());
    assert!(matches!(
        search_catalog(&store, "é".repeat(257)).await,
        Err(Error::InvalidInput)
    ));
    let error = search_catalog(&store, "Visible".into())
        .await
        .err()
        .unwrap();
    let wire = serde_json::to_value(&error).unwrap();
    assert_eq!(wire["code"], "plugin_host_package_unavailable");
    assert_eq!(wire["packageKey"], key);
    assert_eq!(wire["phase"], "admission");
    for field in [
        "resourceId",
        "resourceVersion",
        "instanceId",
        "requestId",
        "nativeRequestId",
    ] {
        assert!(wire.get(field).is_none());
    }
    assert_eq!(
        serde_json::to_value(Error::Unavailable).unwrap(),
        "unavailable"
    );
    assert_eq!(
        serde_json::to_value(Error::SourceUnavailable).unwrap(),
        "sourceUnavailable"
    );
}

#[tokio::test]
async fn runtime_catalog_requires_catalog_capability_and_exact_inventory_bytes() {
    let (root, service, key) = package(false);
    let missing_capability = store(database(), root.path(), service, key);
    let wire = serde_json::to_value(
        search_catalog(&missing_capability, "".into())
            .await
            .err()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(wire["code"], "plugin_host_capability_unavailable");

    let (root, service, _) = package(true);
    fs::write(
        root.path()
            .join("literature/data/context-catalog-papers.v1.sql"),
        "select 'PRIVATE_SQL','unsafe'",
    )
    .unwrap();
    let catalog = RuntimePluginCatalog::new(Some(root.path().into()));
    let key = catalog.discover().unwrap().packages.remove(0).package_key;
    let store = store(database(), root.path(), service, key);
    let wire =
        serde_json::to_value(search_catalog(&store, "".into()).await.err().unwrap()).unwrap();
    assert_eq!(wire["code"], "runtime_plugin_data_digest_mismatch");
    assert_eq!(wire["phase"], "resource");
    assert_eq!(wire["resourceId"], "literature.context-catalog.papers");
    assert_eq!(wire["resourceVersion"], 1);
    assert!(!wire.to_string().contains("PRIVATE_SQL"));
}

#[tokio::test]
async fn runtime_catalog_two_column_codecs_and_safe_sql_errors_keep_read_order() {
    let (_root, service, key) = package(true);
    let resources = service.context_catalog_resources(&key).await.unwrap();
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "create table bibliographic_items(id,title,abstract_text,trashed_at,updated_at);
        create table attachments(id,item_id,display_name,attachment_kind,created_at);
        insert into bibliographic_items values('paper','Needle','','now','now');
        insert into attachments values('pdf','paper','Synthetic.pdf','pdf','now');",
    )
    .unwrap();
    assert!(resources.search_papers(&db, "Needle").unwrap().is_empty());
    db.execute("update bibliographic_items set trashed_at=null", [])
        .unwrap();
    assert_eq!(
        resources.search_papers(&db, "Needle").unwrap(),
        [("paper".into(), "Needle".into())]
    );
    db.execute("update attachments set display_name=x'50524956415445'", [])
        .unwrap();
    let wire = serde_json::to_value(Error::from(
        resources.list_pdf_attachments(&db, "paper").err().unwrap(),
    ))
    .unwrap();
    assert_eq!(
        wire["resourceId"],
        "literature.context-catalog.pdf-attachments"
    );
    assert_eq!(wire["phase"], "read");
    assert!(!wire.to_string().contains("PRIVATE"));
    db.execute("drop table bibliographic_items", []).unwrap();
    let wire: Value = serde_json::to_value(Error::from(
        resources.search_papers(&db, "Needle").err().unwrap(),
    ))
    .unwrap();
    assert_eq!(wire["packageKey"], key);
    assert_eq!(wire["resourceId"], "literature.context-catalog.papers");
    assert!(wire.get("sqliteCode").is_some());
    assert!(!wire.to_string().contains("select id,title"));
    assert!(db.is_autocommit());
}
